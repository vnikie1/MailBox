//! Reading a real `.pst`. docs/06 Phase 11.
//!
//! ## Why this is a gate and not a unit test
//!
//! `transfer::pst`'s unit tests cover the parts that are pure logic — the 1601 epoch, an
//! Exchange distinguished name that is not an address, a newline that must not inject a header.
//! None of them opens a file, so none of them would notice if the store reader stopped working
//! or the folder walk never descended.
//!
//! ## The fixture, and its limits
//!
//! `Empty.pst` comes from Microsoft's own `outlook-pst` repository, which is the only PST this
//! project has that was produced by something other than itself. It is a genuine Unicode PST
//! written by Outlook, with the standard folder tree and no mail in it.
//!
//! That makes this an honest but partial test: it proves the file opens, the store is read, the
//! folder hierarchy is walked, and nothing panics on a real binary — which is the class of
//! failure that would otherwise reach a user's twenty-year archive. It proves **nothing** about
//! message extraction, because there are no messages in it.
//!
//! **The message half is therefore unverified against real Outlook output.** That is written
//! here, in the changelog, and on the import screen, rather than left for somebody to discover.
//! The gap closes when there is a PST with mail in it to test against; the fixture cannot be
//! synthesised honestly, because a file this project wrote would only prove it can read itself.

use std::path::PathBuf;

use halcyon_lib::transfer::pst;

fn fixture() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests")
        .join("fixtures")
        .join("Empty.pst")
}

#[test]
fn gate_1_a_real_pst_opens_and_is_walked_without_panicking() {
    // The failure this exists to catch: a binary format read wrongly does not return an error,
    // it reads garbage — and the first place that shows up should not be somebody's archive.
    let mut found = Vec::new();

    let counts = pst::read(&fixture(), |message| {
        found.push(message);
        Ok(())
    })
    .expect("a valid PST should open");

    assert!(
        counts.folders > 0,
        "no folders were found in a PST that has a folder tree"
    );

    // Empty.pst is empty of mail, by name and by construction.
    assert_eq!(counts.messages, 0);
    assert!(found.is_empty());
}

#[test]
fn gate_2_the_folder_tree_is_walked_rather_than_only_its_root() {
    // A walk that stopped at the root would still report `folders > 0` and look like it worked.
    // The standard Outlook tree has several folders under the message store.
    let counts = pst::read(&fixture(), |_| Ok(())).expect("open");

    assert!(
        counts.folders >= 2,
        "only {} folder(s) found — the walk is not descending",
        counts.folders
    );
}

#[test]
fn gate_3_a_file_that_is_not_a_pst_is_refused_rather_than_misread() {
    // Somebody will point this at an mbox, or a zip, or a renamed .ost. Reading one as a PST
    // must fail loudly; the alternative is inventing folders and messages out of noise.
    let scratch = std::env::temp_dir().join("halcyon-pst-gate-notapst.pst");
    std::fs::write(
        &scratch,
        b"From a@x Thu Jan  1 00:00:00 1970\nSubject: not a pst\n",
    )
    .expect("write");

    let outcome = pst::read(&scratch, |_| Ok(()));
    assert!(outcome.is_err(), "a non-PST file was accepted");

    let _ = std::fs::remove_file(&scratch);
}

#[test]
fn gate_4_a_missing_file_is_an_error_and_not_an_empty_import() {
    // "Imported 0 messages" and "that file is not there" must not look the same to the caller.
    let outcome = pst::read(&PathBuf::from("C:/nowhere/absent.pst"), |_| Ok(()));
    assert!(outcome.is_err());
}
