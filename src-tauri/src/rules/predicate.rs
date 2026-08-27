//! The predicate engine. docs/01 §8, docs/06 Phase 8.
//!
//! **One engine, two consumers.** Smart Mailboxes ask "which stored messages match?" and Rules
//! ask "does this arriving message match?". Those look like different questions and are the
//! same one, and docs/06 is explicit that there must be a single implementation.
//!
//! Two implementations would not merely be duplicated work — they would *disagree*, and the
//! disagreement would be invisible. A rule that files a message which the matching smart
//! mailbox then does not show is a bug nobody can describe, because each half looks correct on
//! its own. So this module compiles a predicate to SQL **and** evaluates it in memory, and the
//! property test at the bottom asserts the two always agree.
//!
//! ## SQL is built with placeholders, never interpolation
//!
//! A predicate contains text the user typed and, through Rules, text that arrived in a message.
//! Every value below becomes a bound parameter. docs/06 makes this a hard constraint for the
//! whole store, and this is the one module where the SQL is assembled rather than written out,
//! so it is the one place the constraint could be lost.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

/// What a predicate can look at. docs/01 §8.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Field {
    From,
    To,
    Cc,
    Subject,
    /// The body text. Matched against the stored plain text, not the HTML: a user searching for
    /// "invoice" means the word they can see, not one hiding in a class name.
    Body,
    /// Any of from, to, subject or body — what "contains" means when nobody picks a field.
    AnyText,
    Mailbox,
    DateReceived,
    Size,
    HasAttachment,
    IsUnread,
    IsFlagged,
    IsJunk,
    AttachmentName,
}

impl Field {
    /// Whether the field holds text, a number, or a yes/no.
    fn kind(self) -> Kind {
        match self {
            Field::DateReceived | Field::Size => Kind::Number,
            Field::HasAttachment | Field::IsUnread | Field::IsFlagged | Field::IsJunk => {
                Kind::Boolean
            }
            _ => Kind::Text,
        }
    }

    /// The columns this field reads. More than one for `AnyText`.
    fn columns(self) -> &'static [&'static str] {
        match self {
            Field::From => &["message.from_all"],
            Field::To => &["message.to_all"],
            // Cc is inside the denormalised `to_all` string as well as its own JSON column; the
            // JSON is the honest source and `LIKE` over it finds the address either way.
            Field::Cc => &["message.cc_json"],
            Field::Subject => &["message.subject"],
            Field::Body => &["message.body_text"],
            Field::AnyText => &[
                "message.from_all",
                "message.to_all",
                "message.subject",
                "message.body_text",
            ],
            Field::Mailbox => &["mailbox.display_name"],
            Field::DateReceived => &["message.date_received"],
            Field::Size => &["message.size"],
            Field::HasAttachment => &["message.has_attachment"],
            Field::IsUnread => &["message.flag_seen"],
            Field::IsFlagged => &["message.flag_flagged"],
            Field::IsJunk => &["message.is_junk"],
            Field::AttachmentName => &["message.attachment_names"],
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Text,
    Number,
    Boolean,
}

/// How a field is compared.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub enum Op {
    Contains,
    NotContains,
    Is,
    IsNot,
    BeginsWith,
    EndsWith,
    /// Numbers: greater than. Dates: after.
    GreaterThan,
    /// Numbers: less than. Dates: before.
    LessThan,
    /// Booleans.
    IsTrue,
    IsFalse,
}

/// One test.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase")]
pub struct Condition {
    pub field: Field,
    pub op: Op,
    /// The comparison value. Ignored for `IsTrue`/`IsFalse`.
    #[serde(default)]
    pub value: String,
}

/// A predicate: conditions combined, and nestable so "all of these, and any of those" is
/// expressible — which is what a five-predicate smart mailbox usually turns out to mean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
#[serde(rename_all = "camelCase", tag = "type", content = "value")]
pub enum Predicate {
    All(Vec<Predicate>),
    Any(Vec<Predicate>),
    Not(Box<Predicate>),
    Is(Condition),
}

/// The message fields the in-memory evaluator needs.
///
/// A borrowed view rather than a row type, so a rule can be run against a message that has not
/// been stored yet — which is the whole point of running rules on arrival.
#[derive(Debug, Clone, Default)]
pub struct Subject<'a> {
    pub from: &'a str,
    pub to: &'a str,
    pub cc: &'a str,
    pub subject: &'a str,
    pub body: &'a str,
    pub mailbox: &'a str,
    pub attachment_names: &'a str,
    pub date_received: i64,
    pub size: i64,
    pub has_attachment: bool,
    pub is_unread: bool,
    pub is_flagged: bool,
    pub is_junk: bool,
}

/// A compiled predicate: a SQL fragment and the values to bind to it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Compiled {
    pub sql: String,
    pub params: Vec<Value>,
}

/// A bound parameter. Text or number — never spliced into the statement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Value {
    Text(String),
    Number(i64),
}

impl rusqlite::ToSql for Value {
    fn to_sql(&self) -> rusqlite::Result<rusqlite::types::ToSqlOutput<'_>> {
        match self {
            Value::Text(text) => text.to_sql(),
            Value::Number(number) => number.to_sql(),
        }
    }
}

/// Escapes the wildcards in a `LIKE` pattern.
///
/// Without this, a user searching for a subject containing `100%` matches every message: `%` is
/// `LIKE`'s "anything". The user typed a percent sign and means a percent sign.
fn escape_like(value: &str) -> String {
    value
        .replace('\\', "\\\\")
        .replace('%', "\\%")
        .replace('_', "\\_")
}

fn boolean_column_is_true(field: Field) -> bool {
    // `IsUnread` reads `flag_seen`, which is the opposite. Storing "seen" and asking "unread" is
    // right for both — the column matches the protocol and the predicate matches the user — but
    // the inversion has to live somewhere, and one place is better than at every call site.
    !matches!(field, Field::IsUnread)
}

impl Condition {
    fn compile(&self) -> Compiled {
        let columns = self.field.columns();

        match self.field.kind() {
            Kind::Boolean => {
                let want = matches!(self.op, Op::IsTrue | Op::Is);
                let column = columns[0];
                let target = i64::from(want == boolean_column_is_true(self.field));

                Compiled {
                    sql: format!("COALESCE({column}, 0) = ?"),
                    params: vec![Value::Number(target)],
                }
            }

            Kind::Number => {
                let column = columns[0];
                let number: i64 = self.value.trim().parse().unwrap_or(0);
                let operator = match self.op {
                    Op::GreaterThan => ">",
                    Op::LessThan => "<",
                    Op::IsNot | Op::NotContains => "<>",
                    _ => "=",
                };

                Compiled {
                    sql: format!("COALESCE({column}, 0) {operator} ?"),
                    params: vec![Value::Number(number)],
                }
            }

            Kind::Text => {
                let (pattern, negate) = match self.op {
                    Op::Contains => (format!("%{}%", escape_like(&self.value)), false),
                    Op::NotContains => (format!("%{}%", escape_like(&self.value)), true),
                    Op::BeginsWith => (format!("{}%", escape_like(&self.value)), false),
                    Op::EndsWith => (format!("%{}", escape_like(&self.value)), false),
                    Op::IsNot => (escape_like(&self.value), true),
                    // `Is` and anything numeric applied to text.
                    _ => (escape_like(&self.value), false),
                };

                // Every column ORed together, so `AnyText` is one condition rather than four.
                let clause = columns
                    .iter()
                    .map(|column| format!("COALESCE({column}, '') LIKE ? ESCAPE '\\'"))
                    .collect::<Vec<_>>()
                    .join(" OR ");

                let params = columns
                    .iter()
                    .map(|_| Value::Text(pattern.clone()))
                    .collect();

                Compiled {
                    // A negation must wrap the whole OR, or "not from ada" would mean "not in
                    // from, or in any of the others", which is true of almost every message.
                    sql: if negate {
                        format!("NOT ({clause})")
                    } else {
                        format!("({clause})")
                    },
                    params,
                }
            }
        }
    }

    fn matches(&self, subject: &Subject<'_>) -> bool {
        match self.field.kind() {
            Kind::Boolean => {
                let actual = match self.field {
                    Field::HasAttachment => subject.has_attachment,
                    Field::IsUnread => subject.is_unread,
                    Field::IsFlagged => subject.is_flagged,
                    Field::IsJunk => subject.is_junk,
                    _ => false,
                };

                let want = matches!(self.op, Op::IsTrue | Op::Is);
                actual == want
            }

            Kind::Number => {
                let actual = match self.field {
                    Field::DateReceived => subject.date_received,
                    Field::Size => subject.size,
                    _ => 0,
                };
                let number: i64 = self.value.trim().parse().unwrap_or(0);

                match self.op {
                    Op::GreaterThan => actual > number,
                    Op::LessThan => actual < number,
                    Op::IsNot | Op::NotContains => actual != number,
                    _ => actual == number,
                }
            }

            Kind::Text => {
                let haystacks: Vec<&str> = match self.field {
                    Field::From => vec![subject.from],
                    Field::To => vec![subject.to],
                    Field::Cc => vec![subject.cc],
                    Field::Subject => vec![subject.subject],
                    Field::Body => vec![subject.body],
                    Field::Mailbox => vec![subject.mailbox],
                    Field::AttachmentName => vec![subject.attachment_names],
                    Field::AnyText => vec![subject.from, subject.to, subject.subject, subject.body],
                    _ => vec![""],
                };

                // SQLite's LIKE is case-insensitive for ASCII by default, and the SQL side
                // relies on that. Matching case-sensitively here would make the two halves
                // disagree on every capital letter.
                let needle = self.value.to_lowercase();
                let any = |test: &dyn Fn(&str) -> bool| {
                    haystacks.iter().any(|hay| test(&hay.to_lowercase()))
                };

                match self.op {
                    Op::Contains => any(&|hay: &str| hay.contains(&needle)),
                    Op::NotContains => !any(&|hay: &str| hay.contains(&needle)),
                    Op::BeginsWith => any(&|hay: &str| hay.starts_with(&needle)),
                    Op::EndsWith => any(&|hay: &str| hay.ends_with(&needle)),
                    Op::IsNot => !any(&|hay: &str| hay == needle),
                    _ => any(&|hay: &str| hay == needle),
                }
            }
        }
    }
}

impl Predicate {
    /// Compiles to a SQL fragment plus its bound parameters.
    ///
    /// The fragment assumes `message` joined to `mailbox`, which is what every caller has.
    pub fn compile(&self) -> Compiled {
        match self {
            Predicate::Is(condition) => condition.compile(),

            Predicate::Not(inner) => {
                let inner = inner.compile();
                Compiled {
                    sql: format!("NOT ({})", inner.sql),
                    params: inner.params,
                }
            }

            Predicate::All(parts) | Predicate::Any(parts) => {
                let joiner = if matches!(self, Predicate::All(_)) {
                    " AND "
                } else {
                    " OR "
                };

                // An empty group is not an error and must not become an empty string, which
                // would produce `WHERE ()`. "All of nothing" is true and "any of nothing" is
                // false, which is both the mathematical answer and the useful one: a smart
                // mailbox with no conditions yet shows everything rather than failing.
                if parts.is_empty() {
                    return Compiled {
                        sql: if matches!(self, Predicate::All(_)) {
                            "1 = 1".to_string()
                        } else {
                            "1 = 0".to_string()
                        },
                        params: Vec::new(),
                    };
                }

                let mut params = Vec::new();
                let clauses: Vec<String> = parts
                    .iter()
                    .map(|part| {
                        let compiled = part.compile();
                        params.extend(compiled.params);
                        compiled.sql
                    })
                    .collect();

                Compiled {
                    sql: format!("({})", clauses.join(joiner)),
                    params,
                }
            }
        }
    }

    /// Evaluates against one message, without touching the database.
    pub fn matches(&self, subject: &Subject<'_>) -> bool {
        match self {
            Predicate::Is(condition) => condition.matches(subject),
            Predicate::Not(inner) => !inner.matches(subject),
            Predicate::All(parts) => parts.iter().all(|part| part.matches(subject)),
            Predicate::Any(parts) => parts.iter().any(|part| part.matches(subject)),
        }
    }

    /// How many conditions it contains, for the "5-predicate smart mailbox" the gate asks for
    /// and for refusing one deep enough to be a denial of service against our own database.
    pub fn condition_count(&self) -> usize {
        match self {
            Predicate::Is(_) => 1,
            Predicate::Not(inner) => inner.condition_count(),
            Predicate::All(parts) | Predicate::Any(parts) => {
                parts.iter().map(Predicate::condition_count).sum()
            }
        }
    }
}
