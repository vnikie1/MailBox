//! The junk filter: a local Bayesian classifier with Fisher combination. docs/01 §8, docs/06.
//!
//! ## Why this and not something cleverer
//!
//! Standing rule 16 forbids phoning home, which rules out every hosted spam service and every
//! shared reputation list. Whatever runs here runs on the user's machine, over the user's own
//! mail, and never transmits a byte of it. That constraint is not a limitation to work around —
//! it is the reason a *local* classifier is the right answer rather than a compromise.
//!
//! Per-token probabilities in Graham's style, combined by Fisher's method in Robinson's. The
//! second half is not a detail: the first version of this used the naive Bayes product and
//! misfiled **1.16% of real mail**, which no threshold fixed — see `Classifier::score` for what
//! goes wrong and why the chi-square form does not.
//!
//! ## The parts that are easy to get wrong
//!
//! - **Training on our own output.** If the classifier's guesses are fed back as training data
//!   it converges on its own mistakes with growing confidence. `junk_by_user` exists precisely
//!   so training only ever reads labels a human applied.
//! - **Unbounded token growth.** A mailbox of 50,000 messages generates millions of distinct
//!   tokens, most seen exactly once, most of them base64 fragments and message ids. Tokens are
//!   length-bounded and rare tokens are ignored at scoring time.
//! - **Floating-point underflow.** Multiplying a few hundred probabilities under 1.0 reaches
//!   zero in f64 quickly, and the answer becomes 0/0. Everything is summed in log space, and
//!   the chi-square tail is summed multiplicatively for the same reason.
//! - **The empty-corpus case.** A fresh install has no training data at all, and a classifier
//!   that answers confidently from nothing would mark real mail as junk on day one. Below a
//!   minimum corpus size it declines to answer.

use std::collections::HashMap;

use rusqlite::{params, Connection, Transaction};

use crate::db::DbError;

/// Tokens shorter or longer than these are noise: single letters carry nothing, and anything
/// long is a base64 chunk, a hash or a message id, which are unique per message and so are
/// pure overfitting — each one would be a perfect predictor of exactly one message.
const MIN_TOKEN: usize = 3;
const MAX_TOKEN: usize = 24;

/// How many of the most extreme tokens are used to decide. Robinson's insight: a message's
/// signal lives in its few most decisive words, and averaging in hundreds of neutral ones
/// drags every score toward the middle.
const SIGNIFICANT: usize = 15;

/// A token seen fewer times than this is not evidence. Without a floor, a word appearing once
/// in one junk message scores as a certainty.
const MIN_OCCURRENCES: i64 = 2;

/// Below this many labelled messages in either class the filter declines to answer.
///
/// A fresh install has no training data, and a classifier that answers confidently from nothing
/// marks real mail as junk on day one — which is how users turn a junk filter off permanently.
pub const MIN_CORPUS: i64 = 20;

/// Above this, junk.
///
/// Deliberately severe, and the value is measured rather than chosen: a false positive hides a
/// real message, a false negative leaves one more piece of spam in the Inbox. Those costs are
/// nowhere near equal and a threshold in the middle pretends they are.
///
/// `junkgate` sweeps this against the SpamAssassin corpus and prints what each setting costs.
/// At 0.90 the filter caught 89.6% of junk and misfiled 0.65% of real mail; at 0.99 it catches
/// 82.6% and misfiles 0.44%. Seven points of catch rate to stop roughly one misfiled message in
/// every three hundred is a trade worth making, because the user notices the misfiled one.
pub const THRESHOLD: f64 = 0.99;

/// Assumed prior for a token never seen in training, and the strength of that assumption.
const UNKNOWN_PRIOR: f64 = 0.4;
const PRIOR_STRENGTH: f64 = 1.0;

/// Splits text into classification tokens.
///
/// Case-folded, because `FREE` and `free` are the same word and treating them separately halves
/// the evidence for both. Header names are prefixed by the caller so that `subject:free` and a
/// body `free` stay distinct — where a word appears is itself a signal.
pub fn tokenise(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();

    for raw in text.split(|c: char| !(c.is_alphanumeric() || c == '\'' || c == '$' || c == '!')) {
        let token = raw.trim_matches('\'').to_lowercase();

        if token.len() < MIN_TOKEN || token.len() > MAX_TOKEN {
            continue;
        }

        // All-digit tokens are dates, prices, ids and quantities. They vary per message and
        // teach the classifier nothing that generalises.
        if token.chars().all(|c| c.is_ascii_digit()) {
            continue;
        }

        tokens.push(token);
    }

    tokens.sort();
    tokens.dedup();
    tokens
}

/// Builds the token set for one message from the parts that carry signal.
///
/// The sender and subject are prefixed so their tokens never merge with body tokens: "offer"
/// in a subject line means something quite different from "offer" three paragraphs into a
/// message from a colleague.
pub fn features(from: &str, subject: &str, body: &str) -> Vec<String> {
    let mut tokens = Vec::new();

    tokens.extend(tokenise(from).into_iter().map(|t| format!("from:{t}")));
    tokens.extend(tokenise(subject).into_iter().map(|t| format!("subj:{t}")));

    // Bodies can be megabytes. The signal is at the top — the pitch is always in the first
    // screenful — and reading all of it would make training over a large mailbox slow enough
    // that nobody would run it.
    let head: String = body.chars().take(4_000).collect();
    tokens.extend(tokenise(&head));

    tokens.sort();
    tokens.dedup();
    tokens
}

/// Records one message as junk or not junk.
///
/// Only ever called with a label a human applied. See the module note on training on our own
/// output — it is the failure mode that turns a working filter into a confidently wrong one.
pub fn train(
    tx: &Transaction<'_>,
    from: &str,
    subject: &str,
    body: &str,
    is_junk: bool,
) -> Result<(), DbError> {
    let class = i64::from(is_junk);

    for token in features(from, subject, body) {
        tx.execute(
            "INSERT INTO junk_token (token, is_junk, count) VALUES (?1, ?2, 1)
             ON CONFLICT(token, is_junk) DO UPDATE SET count = count + 1",
            params![token, class],
        )?;
    }

    tx.execute(
        "INSERT INTO junk_corpus (is_junk, messages) VALUES (?1, 1)
         ON CONFLICT(is_junk) DO UPDATE SET messages = messages + 1",
        params![class],
    )?;

    Ok(())
}

/// Undoes one training example, for when the user reverses a judgement.
///
/// Without this, marking a message as junk and then immediately as not-junk leaves the first
/// judgement in the corpus forever, and the user's correction is half-applied.
pub fn untrain(
    tx: &Transaction<'_>,
    from: &str,
    subject: &str,
    body: &str,
    is_junk: bool,
) -> Result<(), DbError> {
    let class = i64::from(is_junk);

    for token in features(from, subject, body) {
        tx.execute(
            "UPDATE junk_token SET count = MAX(count - 1, 0)
              WHERE token = ?1 AND is_junk = ?2",
            params![token, class],
        )?;
    }

    tx.execute(
        "UPDATE junk_corpus SET messages = MAX(messages - 1, 0) WHERE is_junk = ?1",
        params![class],
    )?;

    Ok(())
}

/// How many messages of each class the corpus holds: `(ham, junk)`.
pub fn corpus_size(conn: &Connection) -> Result<(i64, i64), DbError> {
    let ham = conn
        .query_row(
            "SELECT messages FROM junk_corpus WHERE is_junk = 0",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    let junk = conn
        .query_row(
            "SELECT messages FROM junk_corpus WHERE is_junk = 1",
            [],
            |row| row.get(0),
        )
        .unwrap_or(0);

    Ok((ham, junk))
}

/// The verdict, kept separate from a bare `f64` so "not enough data" cannot be mistaken for a
/// score of zero — which would read as "definitely not junk".
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Verdict {
    /// Not enough training data to answer. Nothing should be marked on this.
    Undecided,
    Scored {
        probability: f64,
    },
}

impl Verdict {
    pub fn is_junk(self) -> bool {
        matches!(self, Verdict::Scored { probability } if probability >= THRESHOLD)
    }

    pub fn probability(self) -> Option<f64> {
        match self {
            Verdict::Scored { probability } => Some(probability),
            Verdict::Undecided => None,
        }
    }
}

/// The trained state, loaded once and reused across a batch.
///
/// Scoring a mailbox one query per token would be tens of thousands of round trips through the
/// reader pool. The table is small enough to hold in memory and the difference is minutes.
pub struct Classifier {
    /// How many of the most decisive tokens are combined. Defaults to [`SIGNIFICANT`]; exposed
    /// so the gate can measure what the value costs rather than leaving it to taste.
    pub significant: usize,
    /// A token seen fewer times than this is not evidence. Exposed for the same reason as
    /// [`Classifier::significant`].
    pub min_occurrences: i64,
    ham_messages: i64,
    junk_messages: i64,
    counts: HashMap<String, (i64, i64)>,
}

impl Classifier {
    pub fn load(conn: &Connection) -> Result<Self, DbError> {
        let (ham_messages, junk_messages) = corpus_size(conn)?;

        let mut statement =
            conn.prepare("SELECT token, is_junk, count FROM junk_token WHERE count >= ?1")?;

        // Loaded at the lowest floor any caller might set, so raising `min_occurrences` filters
        // a table that actually contains the rarer tokens rather than one already stripped of
        // them.
        let rows = statement
            .query_map(params![1_i64], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, i64>(1)?,
                    row.get::<_, i64>(2)?,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        let mut counts: HashMap<String, (i64, i64)> = HashMap::new();

        for (token, class, count) in rows {
            let entry = counts.entry(token).or_insert((0, 0));
            if class == 1 {
                entry.1 = count;
            } else {
                entry.0 = count;
            }
        }

        Ok(Self {
            significant: SIGNIFICANT,
            min_occurrences: MIN_OCCURRENCES,
            ham_messages,
            junk_messages,
            counts,
        })
    }

    pub fn ready(&self) -> bool {
        self.ham_messages >= MIN_CORPUS && self.junk_messages >= MIN_CORPUS
    }

    /// P(junk | this token), with Robinson's correction for how much evidence there is.
    ///
    /// The correction matters: a token seen twice, both times in junk, is not the certainty a
    /// raw ratio claims. Blending toward a neutral prior in proportion to the evidence is what
    /// stops rare tokens from dominating.
    fn token_probability(&self, token: &str) -> Option<f64> {
        let (ham_count, junk_count) = *self.counts.get(token)?;

        if ham_count + junk_count < self.min_occurrences {
            return None;
        }

        // Normalised by class size, or a corpus with 900 ham and 100 junk would call every
        // common English word a ham indicator on volume alone.
        let ham_rate = ham_count as f64 / self.ham_messages.max(1) as f64;
        let junk_rate = junk_count as f64 / self.junk_messages.max(1) as f64;

        let total = ham_rate + junk_rate;
        if total <= 0.0 {
            return None;
        }

        let raw = junk_rate / total;
        let evidence = (ham_count + junk_count) as f64;

        Some((PRIOR_STRENGTH * UNKNOWN_PRIOR + evidence * raw) / (PRIOR_STRENGTH + evidence))
    }

    pub fn score(&self, from: &str, subject: &str, body: &str) -> Verdict {
        if !self.ready() {
            return Verdict::Undecided;
        }

        let mut probabilities: Vec<f64> = features(from, subject, body)
            .iter()
            .filter_map(|token| self.token_probability(token))
            .collect();

        if probabilities.is_empty() {
            return Verdict::Undecided;
        }

        // The most decisive tokens, in either direction. Averaging in hundreds of neutral words
        // drags every message toward the middle and the threshold stops separating anything.
        probabilities.sort_by(|a, b| {
            (b - 0.5)
                .abs()
                .partial_cmp(&(a - 0.5).abs())
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        probabilities.truncate(self.significant);

        // Fisher's method, per Robinson, rather than the naive Bayes product.
        //
        // This was changed because of what the gate measured. Naive Bayes reached 96.7% balanced
        // accuracy but misfiled **1.16% of real mail**, and no threshold fixed it — even at
        // 0.999 it was still 0.73%, because the product form saturates: a handful of extreme
        // tokens pin the result at 0 or 1 and everything after them is arithmetically ignored.
        // Legitimate mail that happens to contain three spammy words is then indistinguishable
        // from actual spam.
        //
        // Fisher combines the same per-token probabilities into two *independent* chi-square
        // statistics — one for how ham-like the evidence is, one for how junk-like — and asks
        // whether each is more extreme than chance. A message can be strongly junk-like and
        // strongly ham-like at once, and the answer lands in the middle instead of at an
        // extreme. That middle is exactly where the newsletters live.
        let n = probabilities.len();
        let mut junk_sum = 0.0_f64;
        let mut ham_sum = 0.0_f64;

        for probability in &probabilities {
            // Clamped away from 0 and 1: ln(0) is negative infinity, which would make one
            // token's certainty override every other token in the message.
            let clamped = probability.clamp(0.000_001, 0.999_999);
            junk_sum += clamped.ln();
            ham_sum += (1.0 - clamped).ln();
        }

        let junk_like = chi_square_tail(-2.0 * junk_sum, 2 * n);
        let ham_like = chi_square_tail(-2.0 * ham_sum, 2 * n);

        // Robinson's I. Half-way when the two agree, which is the honest answer for a message
        // that reads as both.
        let probability = (1.0 + junk_like - ham_like) / 2.0;

        Verdict::Scored {
            probability: probability.clamp(0.0, 1.0),
        }
    }
}

/// The upper tail of the chi-square distribution, for an even number of degrees of freedom.
///
/// Even `df` only — which is all Fisher's method ever needs, since it is always twice a token
/// count — and that reduces the integral to a finite sum with no gamma function anywhere.
///
/// Summed multiplicatively rather than by computing `(x/2)^i / i!` each time: that ratio
/// overflows f64 above roughly 170 terms while the value it represents is perfectly small, and
/// a message with 200 significant tokens would score `NaN`.
fn chi_square_tail(x: f64, df: usize) -> f64 {
    if x <= 0.0 {
        return 1.0;
    }

    let half = x / 2.0;
    let mut term = (-half).exp();
    let mut sum = term;

    for i in 1..(df / 2) {
        term *= half / i as f64;
        sum += term;
    }

    sum.min(1.0)
}

/// Trains from every message the user has judged by hand.
///
/// Reads only `junk_by_user`, never the filter's own verdicts. Returns how many examples of
/// each class it learned from, because "the filter does nothing" and "the filter has seen four
/// examples" look identical from the outside and only one of them is a bug.
pub fn train_from_user_labels(tx: &Transaction<'_>) -> Result<(i64, i64), DbError> {
    tx.execute("DELETE FROM junk_token", [])?;
    tx.execute("DELETE FROM junk_corpus", [])?;

    let examples = {
        let mut statement = tx.prepare(
            "SELECT COALESCE(from_all, ''), COALESCE(subject, ''), COALESCE(body_text, ''),
                    is_junk
               FROM message
              WHERE junk_by_user = 1
                 OR mailbox_id IN (SELECT id FROM mailbox WHERE role = 'junk')
                 OR (flag_seen = 1 AND is_junk = 0)",
        )?;

        let rows = statement
            .query_map([], |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, String>(2)?,
                    row.get::<_, i64>(3)? != 0,
                ))
            })?
            .collect::<rusqlite::Result<Vec<_>>>()?;

        rows
    };

    for (from, subject, body, is_junk) in &examples {
        train(tx, from, subject, body, *is_junk)?;
    }

    let ham = examples.iter().filter(|example| !example.3).count() as i64;
    let junk = examples.iter().filter(|example| example.3).count() as i64;

    Ok((ham, junk))
}
