/**
 * Join class names, dropping anything falsy.
 *
 * Every primitive composes a base class with state classes, and the alternative — a
 * template literal per call — produces stray double spaces and silently stringifies
 * `undefined` into the class attribute, which then shows up in a Playwright snapshot.
 */
export function cx(...parts: (string | false | null | undefined)[]): string {
  return parts
    .filter((part): part is string => typeof part === 'string' && part.length > 0)
    .join(' ')
}
