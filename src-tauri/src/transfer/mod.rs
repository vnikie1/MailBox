//! Moving mail in and out. docs/06 Phase 11.
//!
//! docs/06 asks for import from a Thunderbird profile, an Outlook PST and mbox, and export to
//! mbox and an `.eml` tree. Four of those five are here.
//!
//! **PST is not**, and the reason is worth writing down rather than leaving as an absence.
//! A `.pst` is not a mail file; it is a MAPI object store — a pair of B-trees over a paged
//! heap, holding folders, message objects, recipient tables and attachment tables as numbered
//! properties. There is no RFC 5322 message anywhere in it to extract. Importing one means
//! reading the store *and* reconstructing a message from `PR_SUBJECT`, `PR_TRANSPORT_MESSAGE_
//! HEADERS` where it exists, a recipients table where it does not, and a body that may be
//! plain, HTML, or RTF compressed with a Microsoft-specific dictionary.
//!
//! Microsoft publish `outlook-pst`, a clean-room MS-PST implementation in Rust, which handles
//! the first half properly. The second half — MAPI properties to a message — is the work, and
//! it is its own piece rather than a corner of this one. It is not started, and claiming
//! otherwise in a UI that then imported nothing would be worse than the gap.
//!
//! What a PST user can do today: Outlook itself exports to `.eml` and can be read over IMAP,
//! and both of those arrive here through paths that exist. That is a workaround, not a
//! substitute, and it is written on the import screen rather than left for them to discover.

pub mod export;
pub mod import;
pub mod mbox;
pub mod thunderbird;
