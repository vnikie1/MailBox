import { useEffect } from 'react'

/**
 * Stops Edge's own context menu from appearing over the app.
 *
 * ## Why this is needed at all
 *
 * WebView2 shows the browser's menu — Back, Reload, Save as, Print, Inspect — on every
 * right-click, because that is what a webview does and nothing here had ever told it not to.
 * In a mail client that menu is not merely wrong, it is a way out of the app: "Save as" writes
 * the running page to disk and "Reload" throws away whatever is open. Tauri has no setting for
 * it, so the page has to refuse the event itself.
 *
 * ## Why text fields are exempt
 *
 * A right-click inside an input or the composer should still offer Cut, Copy, Paste and the
 * spelling suggestions — and those come from the browser, not from us. A page cannot rebuild
 * them: reading the clipboard needs a permission this app deliberately does not ask for, and
 * the spelling corrections are not exposed to script at all. So editable regions keep the
 * native menu, and everywhere else the app answers for itself.
 *
 * ## What this does not cover
 *
 * The message body renders in a sandboxed frame with its own document, and an event inside it
 * never reaches this listener. A right-click on the mail itself still shows the browser's
 * menu. That is a separate job — the menu there ought to be Copy and Open Link rather than
 * either of these — and doing it needs a channel into the frame, so it is left whole rather
 * than half-done.
 */
export function useNativeContextMenu(): void {
  useEffect(() => {
    const onContextMenu = (event: MouseEvent) => {
      // Already handled by a region that put its own menu up. `ContextMenu` calls
      // `preventDefault` in its own handler, and this listener runs after it on the way up.
      if (event.defaultPrevented) return

      const target = event.target
      if (target instanceof HTMLElement && isEditable(target)) return

      event.preventDefault()
    }

    // Not capture: a region with its own menu must get the event first, and this has to see
    // the result of that. Bubbling to the document is what puts this last.
    document.addEventListener('contextmenu', onContextMenu)

    return () => {
      document.removeEventListener('contextmenu', onContextMenu)
    }
  }, [])
}

/**
 * Whether a right-click here should keep the browser's menu.
 *
 * `closest` rather than a check on the target itself, because the click can land on a node
 * inside a rich text editor — a paragraph, a link, a styled span — rather than on the element
 * carrying `contenteditable`.
 */
function isEditable(target: HTMLElement): boolean {
  const editable = target.closest('input, textarea, [contenteditable]')
  if (editable === null) return false

  // A disabled or read-only field has nothing to cut or paste into. Checkboxes, radios and
  // buttons are inputs too, and none of them takes text.
  if (editable instanceof HTMLInputElement) {
    return !editable.disabled && !editable.readOnly && TEXTUAL.has(editable.type)
  }

  if (editable instanceof HTMLTextAreaElement) {
    return !editable.disabled && !editable.readOnly
  }

  return editable.getAttribute('contenteditable') !== 'false'
}

/** Input types that hold text a user can select, cut and paste. */
const TEXTUAL = new Set(['text', 'search', 'email', 'url', 'tel', 'password', 'number'])
