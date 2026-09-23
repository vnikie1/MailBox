/**
 * Wording for the compose window's attachments.
 *
 * In its own module rather than beside `ComposeWindow`, so that file exports components only
 * and Fast Refresh keeps working — the same reason `useAccountsGate` is separate.
 */

/** What to say about dropped items that could not be attached. */
export function cannotAttach(names: string[]): string {
  const what = names.length === 1 ? `“${names[0] ?? ''}” was` : `${String(names.length)} items were`
  return `${what} not attached. Only files can be attached — not folders.`
}
