//! Moving mail in and out. docs/06 Phase 11.
//!
//! docs/06 asks for import from a Thunderbird profile, an Outlook PST and mbox, and export to
//! mbox and an `.eml` tree. All five are here.
//!
//! Thunderbird and mbox are one problem: Thunderbird stores mail as mbox files with no
//! extension, so importing it is finding the files and reading them.
//!
//! **PST is a different problem.** A `.pst` is not a mail file; it is a MAPI object store,
//! holding messages as numbered properties with no RFC 5322 anywhere inside it. `outlook-pst` —
//! Microsoft's own clean-room implementation of MS-PST — reads the store; `pst.rs` turns its
//! properties back into messages. Attachments and RTF-only bodies are not extracted, and both
//! are counted and shown rather than dropped quietly: an archive that imports looking complete
//! and is not is the worst thing this code could do.
//!
//! The PST path is also the least tested, and deliberately says so — see `tests/pst_gate.rs`.
//! The only PST this project has that it did not write itself has no mail in it, so the folder
//! walk is proven against real Outlook output and the message extraction is not.
pub mod export;
pub mod import;
pub mod mbox;
pub mod pst;
pub mod thunderbird;
