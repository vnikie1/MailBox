//! Top Hits ranking. docs/01 §7, docs/06 Phase 9.
//!
//! > BM25 × recency decay × VIP boost × thread-participation boost.
//!
//! ## Why relevance alone is the wrong answer for mail
//!
//! BM25 ranks documents in a corpus. A mailbox is not a corpus — it is a timeline of things
//! that happened to one person, and the thing they are looking for is nearly always recent,
//! from someone they know, or in a conversation they took part in. A pure relevance ranking
//! puts a 2019 newsletter that happens to use the search word four times above this morning's
//! reply from their colleague, and the user concludes search does not work.
//!
//! ## Multiplied, not added
//!
//! Each signal is a multiplier around 1.0, so none of them can dominate on its own and all of
//! them compose. Added weights would need re-tuning every time one was changed, because the
//! scale of each would have to be balanced against the others by hand.
//!
//! BM25's own sign is the first trap: SQLite returns it **negative**, more negative meaning a
//! better match. Using it directly as a multiplier would invert the whole ranking, and the
//! result would look like a plausible order rather than an obviously broken one.

/// How quickly a message stops being "recent".
///
/// A half-life, not a cutoff. At 30 days a message keeps half its score, at 60 a quarter — so
/// an old message can still win on a strong enough match, which a cutoff would forbid. Thirty
/// days is roughly where people stop expecting to find something by scrolling.
pub const RECENCY_HALF_LIFE_DAYS: f64 = 30.0;

/// A message from a VIP is worth this much more.
///
/// Deliberately modest. A VIP boost large enough to outrank relevance would turn every search
/// into "mail from your VIPs, mentioning this maybe", which is not what was asked.
pub const VIP_BOOST: f64 = 1.6;

/// A message in a thread the user replied to.
///
/// The strongest signal after relevance, and the least obvious. Someone searching for a word
/// they discussed is looking for that conversation, and having taken part in it is the best
/// evidence available that it is the one.
pub const PARTICIPATED_BOOST: f64 = 1.8;

/// A flagged message the user marked themselves.
pub const FLAGGED_BOOST: f64 = 1.15;

/// What ranking needs to know about one candidate.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Signals {
    /// SQLite's `bm25()`: negative, more negative is a better match. `0.0` when the query had
    /// no free text and every row matched equally.
    pub bm25: f64,
    pub age_seconds: i64,
    pub from_vip: bool,
    /// The user has sent a message in this thread.
    pub participated: bool,
    pub flagged: bool,
}

impl Default for Signals {
    fn default() -> Self {
        Self {
            bm25: 0.0,
            age_seconds: 0,
            from_vip: false,
            participated: false,
            flagged: false,
        }
    }
}

/// The relevance component, normalised so bigger is better and the scale is bounded.
///
/// `bm25` arrives negative. Negating it makes bigger better; the `1.0 +` keeps a zero score —
/// a structured-only query — from collapsing every product to zero, which would leave the
/// other three signals multiplying nothing at all.
fn relevance(bm25: f64) -> f64 {
    if !bm25.is_finite() {
        // A NaN would compare false against everything and scatter the results into an
        // arbitrary order — the kind of failure that looks like bad ranking rather than a bug.
        return 1.0;
    }

    1.0 + (-bm25).max(0.0)
}

/// Exponential decay on age.
fn recency(age_seconds: i64) -> f64 {
    // Mail dated in the future — a sender with a wrong clock, which is common — is treated as
    // brand new rather than given an enormous boost by a negative age.
    let days = (age_seconds.max(0) as f64) / 86_400.0;
    0.5_f64.powf(days / RECENCY_HALF_LIFE_DAYS)
}

/// The score for one candidate. Higher wins.
pub fn score(signals: &Signals) -> f64 {
    let mut score = relevance(signals.bm25) * recency(signals.age_seconds);

    if signals.from_vip {
        score *= VIP_BOOST;
    }

    if signals.participated {
        score *= PARTICIPATED_BOOST;
    }

    if signals.flagged {
        score *= FLAGGED_BOOST;
    }

    score
}

/// Orders candidates, best first, breaking ties by recency.
///
/// The tiebreak matters more than it looks: with a structured-only query every relevance score
/// is identical, and without a deterministic second key the order would come out of whatever
/// SQLite happened to return, changing between identical searches.
pub fn order<T>(mut items: Vec<(T, Signals)>) -> Vec<T> {
    items.sort_by(|a, b| {
        score(&b.1)
            .partial_cmp(&score(&a.1))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| a.1.age_seconds.cmp(&b.1.age_seconds))
    });

    items.into_iter().map(|(item, _)| item).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    const DAY: i64 = 86_400;

    fn signals(bm25: f64, age_days: i64) -> Signals {
        Signals {
            bm25,
            age_seconds: age_days * DAY,
            ..Signals::default()
        }
    }

    #[test]
    fn a_better_bm25_scores_higher_all_else_equal() {
        // SQLite's bm25 is negative and more negative is better. Using it directly as a
        // multiplier would invert the ranking, and the result would look like a plausible
        // order rather than an obviously broken one.
        assert!(score(&signals(-8.0, 1)) > score(&signals(-2.0, 1)));
    }

    #[test]
    fn recency_decays_by_half_over_the_half_life() {
        let fresh = score(&signals(-5.0, 0));
        let older = score(&signals(-5.0, RECENCY_HALF_LIFE_DAYS as i64));

        assert!((older / fresh - 0.5).abs() < 0.01, "{older} / {fresh}");
    }

    #[test]
    fn this_mornings_reply_beats_a_stronger_match_from_years_ago() {
        // The failure the whole module exists to prevent: a 2019 newsletter that uses the word
        // four times ranking above today's answer, after which the user concludes search does
        // not work.
        let today = signals(-3.0, 0);
        let ancient = signals(-9.0, 365 * 5);

        assert!(score(&today) > score(&ancient));
    }

    #[test]
    fn an_old_message_can_still_win_on_a_strong_enough_match() {
        // A half-life rather than a cutoff. A cutoff would make older mail unfindable by
        // relevance at all, which is worse than ranking it below.
        let old_and_perfect = signals(-40.0, 60);
        let recent_and_weak = signals(-0.5, 0);

        assert!(score(&old_and_perfect) > score(&recent_and_weak));
    }

    #[test]
    fn a_vip_is_promoted_but_cannot_outrank_relevance_on_its_own() {
        // A boost large enough to dominate would turn every search into "mail from your VIPs,
        // mentioning this maybe".
        let plain = signals(-5.0, 1);
        let vip = Signals {
            from_vip: true,
            ..plain
        };

        assert!(score(&vip) > score(&plain));

        let much_better_match = signals(-30.0, 1);
        assert!(
            score(&much_better_match) > score(&vip),
            "the VIP boost overwhelmed a far better match"
        );
    }

    #[test]
    fn having_replied_to_a_thread_promotes_it() {
        let plain = signals(-5.0, 3);
        let mine = Signals {
            participated: true,
            ..plain
        };

        assert!(score(&mine) > score(&plain));
    }

    #[test]
    fn the_boosts_compose_rather_than_compete() {
        // Multiplied, not added, so none dominates and all of them apply.
        let base = signals(-5.0, 2);
        let both = Signals {
            from_vip: true,
            participated: true,
            ..base
        };

        let expected = score(&base) * VIP_BOOST * PARTICIPATED_BOOST;
        assert!((score(&both) - expected).abs() < 1e-9);
    }

    #[test]
    fn a_structured_only_query_still_ranks_by_recency() {
        // Every bm25 is zero. Without the `1.0 +` in `relevance` the product would collapse to
        // zero for every row and the other signals would multiply nothing.
        let newer = signals(0.0, 1);
        let older = signals(0.0, 100);

        assert!(score(&newer) > 0.0);
        assert!(score(&newer) > score(&older));
    }

    #[test]
    fn mail_dated_in_the_future_is_treated_as_new_rather_than_boosted() {
        // A sender with a wrong clock is common. A negative age in the exponent would give the
        // message an enormous score and pin it to the top of every search.
        let future = Signals {
            bm25: -5.0,
            age_seconds: -(365 * DAY),
            ..Signals::default()
        };
        let now = signals(-5.0, 0);

        assert!((score(&future) - score(&now)).abs() < 1e-9);
    }

    #[test]
    fn a_broken_relevance_score_does_not_scatter_the_results() {
        // A NaN compares false against everything, which would shuffle the order arbitrarily —
        // a failure that reads as bad ranking rather than as a bug.
        let broken = Signals {
            bm25: f64::NAN,
            ..signals(0.0, 1)
        };

        assert!(score(&broken).is_finite());
    }

    #[test]
    fn ties_break_by_recency_so_the_same_search_gives_the_same_order() {
        let items = vec![
            ("old", signals(0.0, 10)),
            ("new", signals(0.0, 1)),
            ("middle", signals(0.0, 5)),
        ];

        assert_eq!(order(items), vec!["new", "middle", "old"]);
    }
}
