//! Turning a search into a smart mailbox. docs/06 Phase 9.
//!
//! A saved search has to keep matching what it matched when it was saved. Storing the *text*
//! and re-parsing it later would tie its meaning to whatever the parser does in a future
//! version — add a field, change how a date phrase resolves, and a mailbox somebody built a
//! year ago quietly starts returning something else. So the text is converted once, at the
//! moment of saving, into the same [`Predicate`] rules and smart mailboxes already use.
//!
//! ## What does not survive the conversion, and why that is said out loud
//!
//! The predicate engine matches over stored columns. It has no free-text index, so a search's
//! **free-text terms become `anyText contains` conditions** — a `LIKE` over the denormalised
//! columns rather than an FTS match. That is slower and slightly broader than the search was:
//! `LIKE '%fig%'` matches "configure", where FTS matches whole tokens.
//!
//! Relevance ranking is lost outright. A smart mailbox is a set, not a ranking, and it shows
//! its contents newest-first like every other mailbox.
//!
//! Both are stated here rather than discovered later, because a saved search that returns
//! slightly more than the search did is exactly the kind of difference nobody attributes to a
//! documented conversion.

use crate::rules::predicate::{Condition, Field, Op, Predicate};

use super::query::Query;

fn is(field: Field, op: Op, value: &str) -> Predicate {
    Predicate::Is(Condition {
        field,
        op,
        value: value.to_string(),
    })
}

/// Converts a parsed search into the equivalent predicate.
pub fn as_predicate(query: &Query) -> Predicate {
    let mut all: Vec<Predicate> = Vec::new();

    for term in &query.terms {
        all.push(is(Field::AnyText, Op::Contains, term));
    }

    for value in &query.from {
        all.push(is(Field::From, Op::Contains, value));
    }

    for value in &query.subject {
        all.push(is(Field::Subject, Op::Contains, value));
    }

    for value in &query.mailbox {
        all.push(is(Field::Mailbox, Op::Contains, value));
    }

    // `to:` matched recipients *or* Cc in the search. Preserved as an `Any` group rather than
    // flattened into the surrounding `All`, which would turn "to or cc" into "to and cc" and
    // silently return almost nothing.
    for value in &query.to {
        all.push(Predicate::Any(vec![
            is(Field::To, Op::Contains, value),
            is(Field::Cc, Op::Contains, value),
        ]));
    }

    if let Some(wanted) = query.has_attachment {
        all.push(is(
            Field::HasAttachment,
            if wanted { Op::IsTrue } else { Op::IsFalse },
            "",
        ));
    }

    if let Some(unread) = query.is_unread {
        all.push(is(
            Field::IsUnread,
            if unread { Op::IsTrue } else { Op::IsFalse },
            "",
        ));
    }

    if let Some(flagged) = query.is_flagged {
        all.push(is(
            Field::IsFlagged,
            if flagged { Op::IsTrue } else { Op::IsFalse },
            "",
        ));
    }

    if let Some(junk) = query.is_junk {
        all.push(is(
            Field::IsJunk,
            if junk { Op::IsTrue } else { Op::IsFalse },
            "",
        ));
    }

    // Dates are stored as the absolute moments they resolved to, not as the words that produced
    // them. A smart mailbox saved from "last week" means *that* week for ever after, which is
    // what a saved search is; a rolling window would be a different feature and should be asked
    // for explicitly rather than arrived at by accident.
    if let Some(after) = query.after {
        all.push(is(Field::DateReceived, Op::GreaterThan, &after.to_string()));
    }

    if let Some(before) = query.before {
        all.push(is(Field::DateReceived, Op::LessThan, &before.to_string()));
    }

    if let Some(bytes) = query.larger_than {
        all.push(is(Field::Size, Op::GreaterThan, &bytes.to_string()));
    }

    if let Some(bytes) = query.smaller_than {
        all.push(is(Field::Size, Op::LessThan, &bytes.to_string()));
    }

    Predicate::All(all)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::search::query;

    const NOW: i64 = 1_787_000_000;

    fn convert(text: &str) -> Predicate {
        as_predicate(&query::parse(text, NOW))
    }

    fn conditions(predicate: &Predicate) -> Vec<Condition> {
        match predicate {
            Predicate::All(children) | Predicate::Any(children) => {
                children.iter().flat_map(conditions).collect()
            }
            Predicate::Not(inner) => conditions(inner),
            Predicate::Is(condition) => vec![condition.clone()],
        }
    }

    #[test]
    fn free_text_becomes_an_any_text_condition() {
        let predicate = convert("figures");
        let found = conditions(&predicate);

        assert_eq!(found.len(), 1);
        assert_eq!(found[0].field, Field::AnyText);
        assert_eq!(found[0].value, "figures");
    }

    #[test]
    fn fields_carry_across() {
        let found = conditions(&convert("from:ada subject:figures is:unread"));

        assert!(found
            .iter()
            .any(|c| c.field == Field::From && c.value == "ada"));
        assert!(found
            .iter()
            .any(|c| c.field == Field::Subject && c.value == "figures"));
        assert!(found
            .iter()
            .any(|c| c.field == Field::IsUnread && c.op == Op::IsTrue));
    }

    #[test]
    fn to_stays_an_or_rather_than_becoming_an_and() {
        // Flattening it into the surrounding `All` would turn "to or cc" into "to and cc",
        // which matches almost nothing — and the saved mailbox would look simply empty.
        let predicate = convert("to:ada");

        let Predicate::All(children) = &predicate else {
            panic!("expected an All group");
        };

        assert!(
            children
                .iter()
                .any(|child| matches!(child, Predicate::Any(_))),
            "the to/cc alternative was flattened: {predicate:?}"
        );
    }

    #[test]
    fn a_date_phrase_is_saved_as_the_moment_it_resolved_to() {
        // A saved search means the week it was saved in, not a rolling window. A rolling one
        // would be a different feature, and arriving at it by accident is worse than not
        // having it.
        let predicate = convert("last week");
        let found = conditions(&predicate);

        assert!(found
            .iter()
            .any(|c| c.field == Field::DateReceived && c.op == Op::GreaterThan));
        assert!(found
            .iter()
            .any(|c| c.field == Field::DateReceived && c.op == Op::LessThan));

        // Absolute, not a word.
        for condition in found.iter().filter(|c| c.field == Field::DateReceived) {
            assert!(
                condition.value.parse::<i64>().is_ok(),
                "a date survived as text: {}",
                condition.value
            );
        }
    }

    #[test]
    fn an_empty_search_becomes_a_group_that_matches_everything() {
        // `All` of nothing is true, which is the honest reading of "no conditions". The editor
        // shows it as a smart mailbox with no rules yet rather than one that matches nothing.
        assert_eq!(convert(""), Predicate::All(Vec::new()));
    }
}
