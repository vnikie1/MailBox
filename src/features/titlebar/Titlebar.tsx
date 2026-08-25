import styles from './Titlebar.module.css'

/**
 * The toolbar. docs/01 §1, docs/02 §6.1.
 *
 * macOS Mail merges the titlebar and toolbar into one 52px bar. On Windows this app
 * cannot: the system caption sits above, because letting Windows own the caption strip is
 * the only way to get the Snap Layouts flyout out of a WebView-hosted window (the
 * reasoning is in src-tauri/src/platform/mod.rs). So this is the toolbar alone, and the
 * three caption buttons are drawn by Windows above it.
 *
 * Phase 0 is the empty bar. The toolbar groups — sidebar toggle, delete/archive/junk,
 * reply/reply-all/forward, flag/move, search — arrive in Phase 2 with the actions they
 * perform; there is nothing to be gained from drawing buttons that do not do anything.
 *
 * `data-tauri-drag-region` rather than `-webkit-app-region: drag`: the CSS property is a
 * Chromium app-shell feature that WebView2 does not honour, whereas the attribute is what
 * Tauri v2 implements. Mail lets you drag the window by its toolbar, so this bar does too
 * even though the system caption above is already draggable.
 */
export function Titlebar() {
  return <header className={styles.bar} data-tauri-drag-region />
}
