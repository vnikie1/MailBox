/**
 * OS appearance, applied to the document.
 *
 * The Rust core reads the theme, accent and transparency setting from Windows and pushes
 * them here (docs/03 §8). This module is the only place that touches <html> attributes,
 * so the whole app's theming is one remap in semantic.css.
 */

export type ThemeName = 'light' | 'dark'
export type BackdropKind = 'micaAlt' | 'none'

export interface Appearance {
  theme: ThemeName
  /** OS accent as #RRGGBB, or null when Windows would not report one. */
  accent: string | null
  reduceTransparency: boolean
  /** The material actually on the window, not the one we asked for. */
  backdrop: BackdropKind
}

export const DEFAULT_APPEARANCE: Appearance = {
  theme: 'light',
  accent: null,
  reduceTransparency: false,
  backdrop: 'none',
}

/**
 * The user's overrides. Windows reports what the OS wants; these say whether to obey it.
 *
 * Theme and transparency both default to following the OS, which is the macOS behaviour
 * and what docs/01 §11 asks for — but Mail also lets you pin an appearance, and docs/02
 * §5 requires a Reduce Transparency toggle of our own for weak GPUs, whose users may not
 * want the system-wide setting changed. Density has no OS equivalent at all.
 */
export type ThemePreference = 'system' | ThemeName
export type Density = 'compact' | 'default' | 'comfortable'
export type TransparencyPreference = 'system' | 'reduce' | 'full'

export interface DisplayPreferences {
  theme: ThemePreference
  density: Density
  transparency: TransparencyPreference
}

export const DEFAULT_PREFERENCES: DisplayPreferences = {
  theme: 'system',
  density: 'default',
  transparency: 'system',
}

export function resolveTheme(appearance: Appearance, preferences: DisplayPreferences): ThemeName {
  return preferences.theme === 'system' ? appearance.theme : preferences.theme
}

export function resolveReduceTransparency(
  appearance: Appearance,
  preferences: DisplayPreferences,
): boolean {
  if (preferences.transparency === 'system') return appearance.reduceTransparency
  return preferences.transparency === 'reduce'
}

/** sRGB channel to linear light, per WCAG 2.x. */
function linearise(channel8Bit: number): number {
  const c = channel8Bit / 255
  return c <= 0.03928 ? c / 12.92 : Math.pow((c + 0.055) / 1.055, 2.4)
}

export function parseHexColour(hex: string): [number, number, number] | null {
  const match = /^#?([0-9a-f]{6})$/i.exec(hex.trim())
  if (!match?.[1]) return null
  const value = Number.parseInt(match[1], 16)
  return [(value >> 16) & 0xff, (value >> 8) & 0xff, value & 0xff]
}

/** WCAG relative luminance, 0 (black) to 1 (white). */
export function relativeLuminance(hex: string): number | null {
  const rgb = parseHexColour(hex)
  if (!rgb) return null
  const [r, g, b] = rgb
  return 0.2126 * linearise(r) + 0.7152 * linearise(g) + 0.0722 * linearise(b)
}

/**
 * Luminance above which the accent needs dark text on it.
 *
 * Not a WCAG-maximising choice, deliberately. Maximising contrast would put BLACK text on
 * Apple blue (5.2:1 against black vs 4.0:1 against white) and destroy the look the whole
 * project exists to reproduce — macOS uses white there, and so must we. What macOS also
 * does is ship white on a yellow accent at ~1.6:1, which is indefensible.
 *
 * 0.5 threads that needle: every accent in the blue/red/purple/indigo/pink range keeps
 * white, and only genuinely light accents (yellow at 0.64) flip to black. It does not
 * make every accent AA — Apple's own palette does not either — but it removes the case
 * where selected text is effectively unreadable.
 */
const DARK_TEXT_LUMINANCE_THRESHOLD = 0.5

export function accentForeground(accent: string): '#000000' | '#FFFFFF' {
  const luminance = relativeLuminance(accent)
  if (luminance === null) return '#FFFFFF'
  return luminance > DARK_TEXT_LUMINANCE_THRESHOLD ? '#000000' : '#FFFFFF'
}

/**
 * Write the resolved appearance onto <html>. Everything downstream is a token remap:
 * semantic.css keys off [data-theme] and [data-reduce-transparency], component.css keys
 * off [data-density], and the two custom properties feed the accent chain.
 *
 * This is deliberately the only function in the app that writes those attributes. Theme,
 * density and transparency each have two inputs — what Windows reports and what the user
 * pinned — and resolving them in one place is what stops the two halves fighting.
 */
export function applyAppearance(
  appearance: Appearance,
  preferences: DisplayPreferences,
  root: HTMLElement,
): void {
  root.dataset.theme = resolveTheme(appearance, preferences)
  root.dataset.density = preferences.density
  root.dataset.backdrop = appearance.backdrop

  if (resolveReduceTransparency(appearance, preferences)) {
    root.dataset.reduceTransparency = ''
  } else {
    delete root.dataset.reduceTransparency
  }

  // Leaving these unset is meaningful: semantic.css falls back to the Apple blue pair,
  // so the fallback value is never duplicated in two places.
  if (appearance.accent) {
    root.style.setProperty('--accent-system', appearance.accent)
    root.style.setProperty('--accent-fg-system', accentForeground(appearance.accent))
  } else {
    root.style.removeProperty('--accent-system')
    root.style.removeProperty('--accent-fg-system')
  }
}

export function applyWindowActive(active: boolean, root: HTMLElement): void {
  // docs/01 §9.11 — the window goes quiet when inactive.
  if (active) {
    delete root.dataset.windowInactive
  } else {
    root.dataset.windowInactive = ''
  }
}
