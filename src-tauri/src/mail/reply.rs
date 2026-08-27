//! Working out who a reply goes to, and what it quotes. docs/01 §6, docs/06 Phase 7.
//!
//! Small, pure, and heavily tested, because the mistakes here are the expensive kind. Getting
//! recipients wrong does not produce an error — it produces a message that reaches the wrong
//! people, and the user finds out from them. Three failures in particular:
//!
//! * **Replying to a list when you meant the sender**, or the reverse. `Reply-To` exists to
//!   redirect replies and honouring it is not optional.
//! * **Copying yourself.** Harmless-looking, and it means every thread you are in fills your
//!   own inbox with your own mail.
//! * **Leaking `Bcc`.** A blind recipient who is carried into the reply is exposed to everyone
//!   on it, having been told they would not be. `Bcc` is never read here at all — not filtered
//!   late, but never consulted, so no future edit can reintroduce it.

use crate::sync::envelope::{Address, Envelope};

/// Which of the three reply shapes is being built.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// To the sender alone.
    Reply,
    /// To the sender plus everyone else who was on it.
    ReplyAll,
    /// To nobody yet; the user chooses.
    Forward,
    /// Passed on **unaltered** to someone else, as if they had been on it originally.
    ///
    /// Not a forward. A forward is a new message from the user that quotes the original; a
    /// redirect keeps the original's `From`, `Date`, `Subject` and body exactly as they were,
    /// so a reply to it goes back to the original sender rather than to the person who passed
    /// it on. RFC 5322 §3.6.6 covers this with the `Resent-*` headers, which is what makes it
    /// honest rather than forgery: the trail says who passed it on and when, while the
    /// authorship stays with whoever wrote it.
    Redirect,
}

/// The recipients a reply should start with.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Recipients {
    pub to: Vec<Address>,
    pub cc: Vec<Address>,
}

/// Case-insensitive address comparison.
///
/// The local part is case-*sensitive* per RFC 5321, and in practice no mail system on earth
/// treats it that way. Comparing case-sensitively would let `Ada@example.com` and
/// `ada@example.com` both end up on a reply, which reads as a bug to everyone who sees it.
fn same(a: &str, b: &str) -> bool {
    a.trim().eq_ignore_ascii_case(b.trim())
}

/// Addresses that must never be replied to.
///
/// Automated senders that nobody reads, and the machinery of mailing lists. Replying to a
/// `no-reply` address is a message into a void; replying to a list's `-bounces` or `-request`
/// address is a message to a robot, sent in public.
fn is_unreplyable(email: &str) -> bool {
    let lowered = email.trim().to_ascii_lowercase();
    let local = lowered.split('@').next().unwrap_or("");

    local.starts_with("no-reply")
        || local.starts_with("noreply")
        || local.starts_with("do-not-reply")
        || local.starts_with("donotreply")
        || local.ends_with("-bounces")
        || local.ends_with("-request")
        || local.ends_with("-owner")
        || local.ends_with("-confirm")
}

/// Adds an address unless it is a duplicate, one of the user's own, or unreplyable.
fn push_unique(
    into: &mut Vec<Address>,
    candidate: &Address,
    mine: &[String],
    seen: &mut Vec<String>,
) {
    let email = candidate.email.trim();

    if email.is_empty() {
        return;
    }
    if seen.iter().any(|existing| same(existing, email)) {
        return;
    }
    if mine.iter().any(|own| same(own, email)) {
        return;
    }

    seen.push(email.to_string());
    into.push(candidate.clone());
}

/// Who a reply is addressed to.
///
/// `mine` is every address the user owns — accounts and aliases both. It is a list rather than
/// one address because a reply-all on a message that reached two of the user's own accounts
/// would otherwise put one of them in the Cc.
pub fn recipients(envelope: &Envelope, kind: Kind, mine: &[String]) -> Recipients {
    if kind == Kind::Forward || kind == Kind::Redirect {
        // Both start empty on purpose. Pre-filling from the original is how a private thread
        // reaches the people who were already on it, which is at best noise and at worst a
        // disclosure. A redirect in particular must not inherit the original recipients: the
        // whole point is to send it to somebody *new*.
        return Recipients::default();
    }

    let mut result = Recipients::default();
    let mut seen: Vec<String> = Vec::new();

    // `Reply-To` wins over `From` when the sender set one. That is the header's entire
    // purpose — a list, a ticketing system, or a person sending from an address they do not
    // read.
    let primary: &[Address] = if envelope.reply_to.is_empty() {
        &envelope.from
    } else {
        &envelope.reply_to
    };

    for address in primary {
        push_unique(&mut result.to, address, mine, &mut seen);
    }

    // Replying to a message that came only from a no-reply address leaves nothing to send to.
    // Better an empty To the user must fill in than a message posted into a void.
    if result.to.is_empty() {
        for address in primary {
            if !is_unreplyable(&address.email) {
                push_unique(&mut result.to, address, mine, &mut seen);
            }
        }
    }

    if kind == Kind::Reply {
        return result;
    }

    // ---- reply-all -------------------------------------------------------------------------
    // Everyone else who was on it, in the Cc, minus the user and minus anything already in the
    // To. **`envelope.bcc` is deliberately not read.** A blind recipient carried into a reply
    // is exposed to everyone on it, having been told they would not be.
    for address in envelope.to.iter().chain(envelope.cc.iter()) {
        if is_unreplyable(&address.email) {
            continue;
        }
        push_unique(&mut result.cc, address, mine, &mut seen);
    }

    result
}

/// The subject line for a reply or forward.
///
/// Prefixes are not stacked. A thread that has been round a few times acquires
/// `Re: Re: Re:` in clients that simply prepend, which is noise in every mailbox it reaches.
pub fn subject(original: &str, kind: Kind) -> String {
    let trimmed = original.trim();
    let prefix = match kind {
        Kind::Reply | Kind::ReplyAll => "Re: ",
        Kind::Forward => "Fwd: ",
        // Deliberately none. A redirect is the original message, and rewriting its subject
        // would be the one visible sign that it had been tampered with.
        Kind::Redirect => return trimmed.to_string(),
    };

    // Recognised case-insensitively, and `Fw:` as well as `Fwd:` — Outlook sends the former.
    let already = {
        let lowered = trimmed.to_ascii_lowercase();
        match kind {
            Kind::Reply | Kind::ReplyAll => lowered.starts_with("re:"),
            Kind::Forward => lowered.starts_with("fwd:") || lowered.starts_with("fw:"),
            Kind::Redirect => true,
        }
    };

    if already {
        return trimmed.to_string();
    }

    // A reply to a forward keeps both, in the order they happened: "Re: Fwd: ...".
    format!("{prefix}{trimmed}")
}

/// The `References` header for a reply. RFC 5322 §3.6.4.
///
/// The parent's `References` plus the parent's own `Message-ID`. This is what every threading
/// implementation on the receiving end reads, including the one in this app — get it wrong and
/// the reply starts a new conversation in the recipient's client, which is the single most
/// visible way a mail client can look amateur.
pub fn references(parent_references: &[String], parent_message_id: Option<&str>) -> Vec<String> {
    let mut chain: Vec<String> = parent_references.to_vec();

    if let Some(id) = parent_message_id {
        let id = id.trim();
        if !id.is_empty() && !chain.iter().any(|existing| existing == id) {
            chain.push(id.to_string());
        }
    }

    // RFC 5322 allows trimming a long chain, and clients do: keep the first (the thread root,
    // which is what most clients group on) and the most recent, which is what the rest use.
    const MAX: usize = 20;
    if chain.len() > MAX {
        let mut trimmed = vec![chain[0].clone()];
        trimmed.extend_from_slice(&chain[chain.len() - (MAX - 1)..]);
        return trimmed;
    }

    chain
}

/// The attribution line above a quoted reply.
///
/// Apple Mail's exact shape: `On 27 Aug 2026, at 09:34, Ada Lovelace <ada@example.test> wrote:`
/// Matching it is not vanity — a reply that quotes in an unfamiliar format is the first thing
/// that makes a client feel like a port of something else.
pub fn attribution(sender: Option<&Address>, when: &str) -> String {
    let who = match sender {
        Some(address) => match &address.name {
            Some(name) if !name.trim().is_empty() => {
                format!("{} <{}>", name.trim(), address.email.trim())
            }
            _ => address.email.trim().to_string(),
        },
        None => "someone".to_string(),
    };

    format!("On {when}, {who} wrote:")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn address(name: Option<&str>, email: &str) -> Address {
        Address {
            name: name.map(str::to_string),
            email: email.to_string(),
        }
    }

    fn envelope() -> Envelope {
        Envelope {
            message_id: Some("<parent@example.test>".into()),
            subject: "The quarterly figures".into(),
            from: vec![address(Some("Ada Lovelace"), "ada@example.test")],
            to: vec![
                address(Some("Me"), "me@halcyon.test"),
                address(Some("Grace Hopper"), "grace@example.test"),
            ],
            cc: vec![address(None, "charles@example.test")],
            bcc: vec![address(None, "secret@example.test")],
            ..Envelope::default()
        }
    }

    fn mine() -> Vec<String> {
        vec!["me@halcyon.test".to_string()]
    }

    fn emails(list: &[Address]) -> Vec<String> {
        list.iter().map(|a| a.email.clone()).collect()
    }

    /* ------------------------------------------------------------------- who it goes to */

    #[test]
    fn a_reply_goes_to_the_sender_and_nobody_else() {
        let result = recipients(&envelope(), Kind::Reply, &mine());

        assert_eq!(emails(&result.to), ["ada@example.test"]);
        assert!(result.cc.is_empty(), "a plain reply must not copy anyone");
    }

    #[test]
    fn reply_all_copies_everyone_else_but_not_me() {
        let result = recipients(&envelope(), Kind::ReplyAll, &mine());

        assert_eq!(emails(&result.to), ["ada@example.test"]);
        assert_eq!(
            emails(&result.cc),
            ["grace@example.test", "charles@example.test"]
        );
        assert!(
            !emails(&result.cc).contains(&"me@halcyon.test".to_string()),
            "replying to myself fills my own inbox with my own mail"
        );
    }

    #[test]
    fn reply_all_never_exposes_a_blind_recipient() {
        // The one that cannot be allowed to regress. A Bcc'd recipient was promised they were
        // invisible; carrying them into a reply tells everyone on the thread they were there.
        let result = recipients(&envelope(), Kind::ReplyAll, &mine());

        let everyone: Vec<String> = emails(&result.to)
            .into_iter()
            .chain(emails(&result.cc))
            .collect();

        assert!(
            !everyone
                .iter()
                .any(|address| address == "secret@example.test"),
            "a Bcc recipient reached the reply: {everyone:?}"
        );
    }

    #[test]
    fn reply_to_wins_over_from() {
        // The header's entire purpose. A list, a ticketing system, or a person sending from an
        // address they do not read.
        let mut envelope = envelope();
        envelope.reply_to = vec![address(Some("The List"), "list@example.test")];

        let result = recipients(&envelope, Kind::Reply, &mine());
        assert_eq!(emails(&result.to), ["list@example.test"]);
    }

    #[test]
    fn my_own_alias_is_recognised_whatever_its_case() {
        // The local part is case-sensitive per RFC 5321 and nothing treats it that way. A
        // comparison that did would put the user on their own reply.
        let mut envelope = envelope();
        envelope.to = vec![address(None, "Me@Halcyon.Test")];
        envelope.cc = vec![];

        let result = recipients(&envelope, Kind::ReplyAll, &mine());
        assert!(
            result.cc.is_empty(),
            "the user's own address, differently cased, reached the reply: {:?}",
            emails(&result.cc)
        );
    }

    #[test]
    fn a_duplicate_address_appears_once() {
        let mut envelope = envelope();
        envelope.cc = vec![
            address(None, "grace@example.test"),
            address(Some("Grace"), "GRACE@example.test"),
        ];

        let result = recipients(&envelope, Kind::ReplyAll, &mine());
        assert_eq!(emails(&result.cc), ["grace@example.test"]);
    }

    #[test]
    fn list_machinery_and_no_reply_addresses_are_not_copied() {
        // Replying to a bounce handler is a message to a robot, sent in public.
        let mut envelope = envelope();
        envelope.cc = vec![
            address(None, "list-bounces@example.test"),
            address(None, "no-reply@example.test"),
            address(None, "list-request@example.test"),
            address(None, "real.person@example.test"),
        ];

        let result = recipients(&envelope, Kind::ReplyAll, &mine());
        assert_eq!(
            emails(&result.cc),
            ["grace@example.test", "real.person@example.test"]
        );
    }

    #[test]
    fn a_forward_starts_with_nobody_on_it() {
        // Pre-filling from the original is how a private thread reaches the people already on
        // it — at best noise, at worst a disclosure.
        let result = recipients(&envelope(), Kind::Forward, &mine());

        assert!(result.to.is_empty());
        assert!(result.cc.is_empty());
    }

    /* ---------------------------------------------------------------------- the subject */

    #[test]
    fn subject_prefixes_do_not_stack() {
        assert_eq!(subject("Hello", Kind::Reply), "Re: Hello");
        assert_eq!(subject("Re: Hello", Kind::Reply), "Re: Hello");
        assert_eq!(subject("RE: Hello", Kind::ReplyAll), "RE: Hello");
        assert_eq!(subject("Fwd: Hello", Kind::Forward), "Fwd: Hello");
        // Outlook sends "Fw:", and a client that does not recognise it produces "Fwd: Fw:".
        assert_eq!(subject("Fw: Hello", Kind::Forward), "Fw: Hello");
    }

    #[test]
    fn replying_to_a_forward_keeps_both_prefixes() {
        assert_eq!(subject("Fwd: Hello", Kind::Reply), "Re: Fwd: Hello");
    }

    /* ------------------------------------------------------------------- the reference chain */

    #[test]
    fn references_extend_the_parents_chain() {
        // What every threading implementation on the receiving end reads. Getting it wrong
        // starts a new conversation in the recipient's client.
        let chain = references(
            &["<root@example.test>".into(), "<second@example.test>".into()],
            Some("<parent@example.test>"),
        );

        assert_eq!(
            chain,
            [
                "<root@example.test>",
                "<second@example.test>",
                "<parent@example.test>"
            ]
        );
    }

    #[test]
    fn a_first_reply_starts_the_chain_with_the_parent() {
        assert_eq!(
            references(&[], Some("<parent@example.test>")),
            ["<parent@example.test>"]
        );
    }

    #[test]
    fn a_long_chain_keeps_the_root_and_the_recent_end() {
        // Clients trim, and RFC 5322 allows it — but the root has to survive, because it is
        // what most clients group the conversation on.
        let long: Vec<String> = (0..40).map(|i| format!("<m{i}@example.test>")).collect();
        let chain = references(&long, Some("<parent@example.test>"));

        assert_eq!(chain.len(), 20);
        assert_eq!(chain[0], "<m0@example.test>", "the root must survive");
        assert_eq!(chain[chain.len() - 1], "<parent@example.test>");
    }

    #[test]
    fn a_parent_already_in_the_chain_is_not_repeated() {
        let chain = references(
            &["<parent@example.test>".into()],
            Some("<parent@example.test>"),
        );
        assert_eq!(chain, ["<parent@example.test>"]);
    }

    /* ------------------------------------------------------------------- the attribution */

    #[test]
    fn the_attribution_line_matches_mails_shape() {
        let line = attribution(
            Some(&address(Some("Ada Lovelace"), "ada@example.test")),
            "27 Aug 2026, at 09:34",
        );

        assert_eq!(
            line,
            "On 27 Aug 2026, at 09:34, Ada Lovelace <ada@example.test> wrote:"
        );
    }

    #[test]
    fn an_attribution_without_a_display_name_uses_the_address_alone() {
        let line = attribution(
            Some(&address(None, "ada@example.test")),
            "27 Aug 2026, at 09:34",
        );
        assert_eq!(line, "On 27 Aug 2026, at 09:34, ada@example.test wrote:");
    }
}
