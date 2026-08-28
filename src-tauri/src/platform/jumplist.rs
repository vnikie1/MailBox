//! The taskbar Jump List. docs/06 Phase 10.
//!
//! Right-click the taskbar button and Windows shows a menu: New Message, Inbox, Search. These
//! are "tasks" — the fixed section every app can define, as opposed to the recent-files section
//! the shell fills in on its own.
//!
//! ## How a task actually works
//!
//! Each one is an `IShellLink` pointing at this executable with an argument, plus a title in the
//! link's property store. Picking one runs `halcyon.exe --inbox`. Because the single-instance
//! plugin is installed, that second process hands its arguments to the running one and exits, so
//! the argument arrives in the same place a `mailto:` does — `links::handle_arguments`. There is
//! one entry point for "the shell is asking for something", which is why the jump list needed no
//! new plumbing beyond the three argument names.
//!
//! ## Why the title goes in a property store rather than a description
//!
//! `SetDescription` is the tooltip. A link with a description and no `PKEY_Title` shows up in the
//! jump list as the *filename* of its target — every task reading "halcyon.exe". The title has to
//! be written as a property, which is the part of this API that is easy to get wrong and looks
//! fine until you actually open the menu.
//!
//! ## What this needs that a dev build does not have
//!
//! Like toasts, a jump list is keyed to the app's taskbar identity. An uninstalled dev build has
//! no registered AUMID, so `CommitList` may succeed and the menu still show nothing. That is the
//! same Phase 12 dependency `toast.rs` documents, and it is not a failure worth surfacing.

use windows::core::{Interface, PCWSTR};
use windows::Win32::Foundation::PROPERTYKEY;
use windows::Win32::System::Com::StructuredStorage::{
    InitPropVariantFromStringVector, PropVariantClear,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::Common::{IObjectArray, IObjectCollection};
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;
use windows::Win32::UI::Shell::{
    DestinationList, EnumerableObjectCollection, ICustomDestinationList, IShellLinkW, ShellLink,
};

/// `PKEY_Title`. Not exported by the `windows` crate, so it is spelled out.
///
/// The GUID is the standard summary-information property set and 2 is the title within it; this
/// is the same key Explorer uses for a document's title, and the shell reads it here for the
/// text of a jump list entry.
const PKEY_TITLE: PROPERTYKEY = PROPERTYKEY {
    fmtid: windows::core::GUID::from_u128(0xf29f85e0_4ff9_1068_ab91_08002b27b3d9),
    pid: 2,
};

/// A task in the menu: what it says, and what argument it launches with.
struct Task {
    title: &'static str,
    argument: &'static str,
}

/// The three docs/06 asks for.
///
/// Ordered by how often they are wanted, not by importance. A jump list is used by muscle memory
/// and the top item is the one people hit without reading — which is why New Message, the only
/// one that cannot lose anything, is first.
const TASKS: &[Task] = &[
    Task {
        title: "New Message",
        argument: "--new-message",
    },
    Task {
        title: "Inbox",
        argument: "--inbox",
    },
    Task {
        title: "Search",
        argument: "--search",
    },
];

/// A null-terminated UTF-16 buffer, since every one of these calls wants one.
fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Builds one task as a shell link.
unsafe fn link_for(task: &Task, exe: &[u16]) -> windows::core::Result<IShellLinkW> {
    let link: IShellLinkW = CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER)?;

    link.SetPath(PCWSTR(exe.as_ptr()))?;
    link.SetArguments(PCWSTR(wide(task.argument).as_ptr()))?;

    // The icon comes from the executable itself. Index 0 is the app icon, so every task shows
    // the Halcyon mark rather than the generic link glyph the shell falls back to.
    link.SetIconLocation(PCWSTR(exe.as_ptr()), 0)?;

    // The title, as a property. See the note at the top of this file: without it every entry
    // in the menu reads "halcyon.exe".
    let store: IPropertyStore = link.cast()?;
    let title = wide(task.title);
    let mut value = InitPropVariantFromStringVector(Some(&[PCWSTR(title.as_ptr())]))?;

    store.SetValue(&PKEY_TITLE, &value)?;
    store.Commit()?;

    let _ = PropVariantClear(&mut value);

    Ok(link)
}

/// Installs the jump list. Called once, after the window exists.
///
/// Every failure here is silent by design. A missing jump list is a menu somebody does not see;
/// it is not worth an error path, and on an uninstalled build it is the expected outcome.
pub fn install() {
    if let Err(error) = build() {
        tracing::debug!(%error, "could not build the jump list");
    }
}

fn build() -> windows::core::Result<()> {
    let exe = wide(
        &std::env::current_exe()
            .map_err(|_| windows::core::Error::from_win32())?
            .to_string_lossy(),
    );

    unsafe {
        let list: ICustomDestinationList =
            CoCreateInstance(&DestinationList, None, CLSCTX_INPROC_SERVER)?;

        // BeginList reports how many slots the user's settings allow and hands back the items
        // they have removed by hand. Both are ignored here — there are three tasks, far below
        // any limit, and the removed-items list only applies to the recent/frequent sections
        // this app does not use. It still has to be called: CommitList fails without it.
        let mut slots: u32 = 0;
        let _removed: IObjectArray = list.BeginList(&mut slots)?;

        let collection: IObjectCollection =
            CoCreateInstance(&EnumerableObjectCollection, None, CLSCTX_INPROC_SERVER)?;

        for task in TASKS {
            collection.AddObject(&link_for(task, &exe)?)?;
        }

        list.AddUserTasks(&collection.cast::<IObjectArray>()?)?;
        list.CommitList()?;
    }

    Ok(())
}

/// Whether an argument is one of ours, and which.
///
/// Public so `links::handle_arguments` can route it, and so the set of names lives in one place
/// rather than being spelled out again at the point they are matched.
pub fn task_for(argument: &str) -> Option<&'static str> {
    TASKS
        .iter()
        .find(|task| task.argument == argument)
        .map(|task| task.argument)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_task_is_recognised_when_it_comes_back() {
        // The argument is written into a shell link in one function and matched in another. A
        // typo in either produces a menu entry that launches the app and then does nothing.
        for task in TASKS {
            assert_eq!(task_for(task.argument), Some(task.argument));
        }
    }

    #[test]
    fn an_unknown_argument_is_not_a_task() {
        assert_eq!(task_for("--wipe-everything"), None);
        assert_eq!(task_for("mailto:a@x.test"), None);
        assert_eq!(task_for(""), None);
    }

    #[test]
    fn the_tasks_are_the_three_the_spec_asks_for() {
        let titles: Vec<&str> = TASKS.iter().map(|task| task.title).collect();
        assert_eq!(titles, vec!["New Message", "Inbox", "Search"]);
    }

    #[test]
    fn wide_strings_are_terminated() {
        // Every one of these is handed to a C API that reads until a null. A missing terminator
        // is not a compile error and not a panic — it is whatever happened to be in memory next.
        let encoded = wide("Inbox");

        assert_eq!(encoded.last(), Some(&0));
        assert_eq!(encoded.len(), "Inbox".len() + 1);
    }
}
