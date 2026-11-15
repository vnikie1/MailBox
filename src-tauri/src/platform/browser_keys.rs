//! WebView2's own browser keys, turned off in every window. M39, 2026-11-14 audit.
//!
//! WebView2 is a browser, and by default it answers a browser's keys: F5 and Ctrl+R reload the
//! page, Ctrl+P prints it, Ctrl+F opens its own find bar, Alt+Left goes back. In this app a reload
//! is a window wiped back to its first frame. In a compose window that is the draft being typed,
//! gone without a question.
//!
//! The main window binds F5 (Get Mail) and Ctrl+R (Reply) itself and swallows them, but only
//! while focus is outside a text field. In the search field, in the message body (a sandboxed
//! frame no script of ours runs in), and in the compose and Settings windows, which bind neither,
//! the keys went to WebView2 and the window reloaded.
//!
//! So they are turned off at the source, for every webview, as each page starts to load. What the
//! app binds itself is unaffected: its keys are handled in the page before WebView2 would have
//! looked. Editing keys (copy, paste, undo, select all) are not browser accelerator keys and keep
//! working. The cost is the developer tools' F12 and Ctrl+Shift+I; the tools themselves are still
//! there in a development build, opened from the code or over the remote debugging port.

use tauri::Webview;
use webview2_com::Microsoft::Web::WebView2::Win32::ICoreWebView2Settings3;
use windows::core::Interface;

/// Turns off the browser keys for one webview. Idempotent, so calling it on every page load is
/// harmless; a failure is logged and the window works as before.
pub fn turn_off(webview: &Webview) {
    let label = webview.label().to_string();

    let scheduled = webview.with_webview(move |platform| {
        // SAFETY: COM calls on the controller Tauri created for this webview, on the thread
        // `with_webview` runs us on, which is the one the webview belongs to.
        let outcome = unsafe {
            platform
                .controller()
                .CoreWebView2()
                .and_then(|core| core.Settings())
                .and_then(|settings| settings.cast::<ICoreWebView2Settings3>())
                .and_then(|settings| settings.SetAreBrowserAcceleratorKeysEnabled(false))
        };

        if let Err(error) = outcome {
            tracing::warn!(%error, window = label, "could not turn off the browser's own keys");
        }
    });

    if let Err(error) = scheduled {
        tracing::warn!(%error, "could not reach the webview to turn off the browser's own keys");
    }
}
