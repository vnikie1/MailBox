//! JWZ threading. docs/03-architecture.md §5.
//!
//! The tests for this are in `threading_tests.rs` and were written first, as
//! `docs/06` Phase 5 requires.
//!
//! Jamie Zawinski's algorithm, with the two adjustments docs/03 §5 specifies: subject
//! grouping applies **only** to root sets with no reference link between them, and Gmail's
//! `X-GM-THRID` overrides the whole thing where it is present.
//!
//! The implementation is union-find rather than JWZ's literal container tree. The tree is the
//! natural expression of the algorithm when you also want to *display* a tree; this app shows
//! a flat conversation ordered by date, so all that is needed is the partition — which set
//! does each message belong to. Union-find gives that in near-linear time, and it makes the
//! two properties that actually matter fall out for free: a bridging message merges two sets
//! by construction, and a reference cycle cannot loop because union-find has no traversal to
//! get stuck in. The `a_reference_cycle_terminates` test is the one that would fail against a
//! parent-pointer walk.

use std::collections::HashMap;

/// What threading needs to know about a message. Deliberately not a database row: threading
/// is pure, and testing it against a table would test SQLite instead.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Threadable {
    pub id: i64,
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    /// Parsed out of the `References` header, oldest first.
    pub references: Vec<String>,
    pub subject: String,
    pub date: i64,
    /// Gmail's `X-GM-THRID`. When present it is authoritative.
    pub gm_thrid: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Assignment {
    pub message_id: i64,
    /// Stable identifier for the thread this message belongs to.
    ///
    /// The smallest message id in the thread, so it does not change between syncs as long as
    /// the membership does not — the `thread_id` column is persisted, and a key that churned
    /// would rewrite every row on every sync.
    pub thread_key: i64,
}

/// Strips reply and forward prefixes. docs/03 §5.
///
/// Case- and whitespace-insensitive, applied repeatedly, and — the part that matters — only
/// when the prefix is genuinely a prefix. "Reminder:" starts with "Re" and is not a reply;
/// stripping it would merge unrelated conversations, which is the kind of wrongness a user
/// notices immediately and never trusts again.
pub fn subject_base(subject: &str) -> String {
    let mut current = subject.trim();

    loop {
        let stripped = strip_one_prefix(current);

        match stripped {
            Some(rest) => current = rest.trim_start(),
            None => break,
        }
    }

    current.trim().to_lowercase()
}

/// One round of prefix removal, or `None` when there is nothing left to strip.
fn strip_one_prefix(subject: &str) -> Option<&str> {
    // A bracketed list tag: "[rust-dev] ..." — but not an unclosed bracket, and not a tag
    // that is the entire subject.
    if let Some(rest) = subject.strip_prefix('[') {
        if let Some(end) = rest.find(']') {
            return Some(&rest[end + 1..]);
        }
    }

    // "Re", "Fwd", "AW" and friends, optionally followed by spaces, then a colon. The space
    // before the colon is real: "RE : subject" is in docs/03 §5 because clients send it.
    const PREFIXES: &[&str] = &["re", "fwd", "fw", "aw", "sv", "vs", "antw", "rif", "res"];

    let lowered = subject.to_ascii_lowercase();

    for prefix in PREFIXES {
        let Some(rest) = lowered.strip_prefix(prefix) else {
            continue;
        };

        // Whatever follows the letters must be optional spaces and then a colon. Without
        // this check "Reminder" strips to "minder" and "AWS bill" to "S bill".
        let after_spaces = rest.trim_start_matches(' ');

        if let Some(tail) = after_spaces.strip_prefix(':') {
            let consumed = subject.len() - tail.len();
            return Some(&subject[consumed..]);
        }
    }

    None
}

/// Disjoint-set forest over message indices, with union by size and path halving.
struct Union {
    parent: Vec<usize>,
    size: Vec<usize>,
}

impl Union {
    fn new(count: usize) -> Self {
        Self {
            parent: (0..count).collect(),
            size: vec![1; count],
        }
    }

    fn find(&mut self, mut node: usize) -> usize {
        while self.parent[node] != node {
            // Path halving: point at the grandparent as we go. Keeps `find` near-constant
            // without the second pass full compression needs.
            let grandparent = self.parent[self.parent[node]];
            self.parent[node] = grandparent;
            node = grandparent;
        }

        node
    }

    fn union(&mut self, left: usize, right: usize) {
        let (mut a, mut b) = (self.find(left), self.find(right));

        if a == b {
            return;
        }

        if self.size[a] < self.size[b] {
            std::mem::swap(&mut a, &mut b);
        }

        self.parent[b] = a;
        self.size[a] += self.size[b];
    }
}

/// Groups messages into threads.
///
/// Every input message appears in the output exactly once. That is the property worth
/// protecting above correctness of the grouping itself: a conversation split in two is
/// annoying, a message that falls out of the partition has disappeared from the mailbox.
pub fn thread_messages(messages: &[Threadable]) -> Vec<Assignment> {
    if messages.is_empty() {
        return Vec::new();
    }

    let mut union = Union::new(messages.len());

    // ---- Gmail first -------------------------------------------------------------------
    // Where Google has already threaded, its answer is the one the user sees in the Gmail
    // web UI, and disagreeing with it makes the same conversation look different in two
    // places. Messages without a thrid fall through to the algorithm below.
    let mut by_thrid: HashMap<i64, usize> = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        if let Some(thrid) = message.gm_thrid {
            match by_thrid.entry(thrid) {
                std::collections::hash_map::Entry::Occupied(seen) => {
                    union.union(*seen.get(), index);
                }
                std::collections::hash_map::Entry::Vacant(slot) => {
                    slot.insert(index);
                }
            }
        }
    }

    // ---- id table ----------------------------------------------------------------------
    // Maps a Message-ID to the message that *claims* it. A duplicate id keeps the first
    // claimant; the second is still threaded through it, and neither is dropped.
    let mut by_message_id: HashMap<&str, usize> = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        if let Some(id) = message.message_id.as_deref().filter(|id| !id.is_empty()) {
            by_message_id.entry(id).or_insert(index);
        }
    }

    // Messages that reference an id we do not hold are still related to each other — JWZ's
    // empty container. Two replies to a deleted message belong together.
    let mut by_missing_reference: HashMap<&str, usize> = HashMap::new();

    for (index, message) in messages.iter().enumerate() {
        if message.gm_thrid.is_some() {
            continue;
        }

        let references = message
            .references
            .iter()
            .map(String::as_str)
            .chain(message.in_reply_to.as_deref())
            .filter(|id| !id.is_empty());

        for reference in references {
            // Self-reference: a broken client, and a parent walk's favourite infinite loop.
            if message.message_id.as_deref() == Some(reference) {
                continue;
            }

            match by_message_id.get(reference) {
                Some(&parent) if parent != index => union.union(parent, index),
                Some(_) => {}
                None => match by_missing_reference.entry(reference) {
                    std::collections::hash_map::Entry::Occupied(seen) => {
                        union.union(*seen.get(), index);
                    }
                    std::collections::hash_map::Entry::Vacant(slot) => {
                        slot.insert(index);
                    }
                },
            }
        }
    }

    // ---- subject grouping is deliberately absent ---------------------------------------
    // docs/03 §5 permits grouping root sets by `subject_base` "only where no reference link
    // exists". In practice that rule cannot be applied safely to a mailbox: with no link,
    // there is nothing distinguishing this year's "Re: lunch?" from the one three years ago,
    // and merging them is the single most damaging thing a mail client can do to a mailbox.
    // `an_identical_subject_alone_never_merges_two_conversations` pins the decision.
    //
    // `subject_base` is still computed and stored, because the `subject_base` column drives
    // the conversation title and the Phase 8 smart-mailbox rules.

    // ---- resolve ------------------------------------------------------------------------
    // The key is the smallest message id in each set: stable across runs, and independent of
    // the order the rows came back from the database.
    let mut key_for_root: HashMap<usize, i64> = HashMap::new();
    for (index, message) in messages.iter().enumerate() {
        let root = union.find(index);
        let id = message.id;

        key_for_root
            .entry(root)
            .and_modify(|current| *current = (*current).min(id))
            .or_insert(id);
    }

    (0..messages.len())
        .map(|index| {
            let root = union.find(index);

            Assignment {
                message_id: messages[index].id,
                // The root is always present: it was inserted by the loop above.
                thread_key: key_for_root
                    .get(&root)
                    .copied()
                    .unwrap_or(messages[index].id),
            }
        })
        .collect()
}
