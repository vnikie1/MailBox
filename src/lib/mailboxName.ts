/**
 * The rules a mailbox name has to meet, checked while it is typed.
 *
 * A mirror of `validate_name` in `src-tauri/src/sync/folders.rs`, in the same words. The core
 * checks again and its answer is the one that counts; this exists so the New and Rename sheets
 * can say what is wrong before the button is pressed rather than after. The browser store uses
 * it too, as the core's stand-in.
 *
 * Whether a name is already *taken* is not checked here. That depends on where the folder goes
 * on the server — a top-level "Clients" and "Work/Clients" are different folders with the same
 * name in the sidebar — and only the core knows the paths. Its refusal is shown in the same
 * place as these.
 */

/** `sync::folders::MAX_NAME`. */
export const MAX_MAILBOX_NAME = 200

/**
 * The sentence that says what is wrong with a name, or null when there is nothing.
 *
 * `delimiter` is the server's hierarchy separator, or null before a sync has recorded it — in
 * which case "/", the commonest, stands in, exactly as it does in the core.
 */
export function mailboxNameProblem(name: string, delimiter: string | null): string | null {
  const trimmed = name.trim()

  if (trimmed === '') return 'Enter a name for the mailbox.'

  // `char::is_control` in Rust is Unicode's Cc category, which is what this matches.
  if (/\p{Cc}/u.test(trimmed)) return 'A mailbox name can’t contain tabs or line breaks.'

  const separator = delimiter ?? '/'
  if (separator !== '' && trimmed.includes(separator)) {
    return `A mailbox name can’t contain “${separator}”.`
  }

  if (trimmed.includes('%') || trimmed.includes('*')) {
    return 'A mailbox name can’t contain “%” or “*”.'
  }

  // Counted in code points, as Rust's `chars().count()` does — not in UTF-16 units, and not in
  // grapheme clusters either, because the two sides have to agree on where the limit is.
  if (Array.from(trimmed).length > MAX_MAILBOX_NAME) {
    return `A mailbox name can be at most ${String(MAX_MAILBOX_NAME)} characters.`
  }

  return null
}
