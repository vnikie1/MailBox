/**
 * What a recipient chip has to look like before the field stops colouring it red.
 *
 * Its own module rather than a helper inside `ComposeWindow`, so it can be tested on its own —
 * the same reason `any_unread` is split out of its command in the core. It is a rule about what
 * an address looks like, and it was wrong in a way no rendering test would have shown.
 */

/**
 * A very loose check. The core validates properly; this only decides whether a chip looks wrong.
 *
 * The angle-bracket form matters and is the whole reason this is not a one-liner. `RecipientField`
 * commits a suggestion as `Ada Lovelace <ada@example.test>` deliberately, so the message carries
 * the name the mailbox already knows, and people paste that form by hand as well. Testing the
 * whole token — which is what this did — meant the display name's space failed the whitespace
 * check, and **every recipient picked from the autocomplete rendered as an invalid red chip**.
 */
export function looksLikeAddress(value: string): boolean {
  const match = /^(.*?)<([^>]+)>\s*$/.exec(value)
  const email = (match ? match[2] : value)?.trim() ?? ''

  const at = email.indexOf('@')
  return at > 0 && at < email.length - 1 && !/\s/.test(email)
}
