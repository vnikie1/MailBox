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

/**
 * The accent colours the app offers, light and dark. docs/01 §11.
 *
 * The same eleven pairs `primitive.css` holds, duplicated here for one reason: the accent has
 * to be resolved to a **value** before first paint, and handed to the taskbar badge, and
 * neither of those can read a CSS custom property. Everything downstream still comes from the
 * token layer — this only decides what `--accent-system` becomes, exactly as the OS accent
 * does today.
 *
 * A duplicated table is a table that drifts, so `tests/unit/accentPalette.test.ts` parses
 * `primitive.css` and fails if the two stop agreeing. The repo already uses that idiom to keep
 * the settings panes aligned with the Rust source.
 *
 * The pairs matter. Mint is the clearest case: #00c7be wants white on it and #63e6e2 wants
 * black, so one value per hue would be unreadable in one theme or the other.
 */
export const ACCENT_PALETTE = {
  blue: { light: '#007aff', dark: '#0a84ff' },
  red: { light: '#ff3b30', dark: '#ff453a' },
  orange: { light: '#ff9500', dark: '#ff9f0a' },
  yellow: { light: '#ffcc00', dark: '#ffd60a' },
  green: { light: '#28cd41', dark: '#32d74b' },
  mint: { light: '#00c7be', dark: '#63e6e2' },
  teal: { light: '#59adc4', dark: '#6ac4dc' },
  indigo: { light: '#5856d6', dark: '#5e5ce6' },
  purple: { light: '#af52de', dark: '#bf5af2' },
  pink: { light: '#ff2d55', dark: '#ff375f' },
  gray: { light: '#8e8e93', dark: '#98989d' },
} as const

export type AccentName = keyof typeof ACCENT_PALETTE

/** Which accent the app draws with: the one Windows reports, or one the user pinned. */
export type AccentPreference = 'system' | AccentName

/** In the order the picker shows them, which is docs/01 §11's order and not alphabetical. */
export const ACCENT_NAMES = Object.keys(ACCENT_PALETTE) as AccentName[]

export function isAccentName(value: string): value is AccentName {
  return value in ACCENT_PALETTE
}

export interface DisplayPreferences {
  theme: ThemePreference
  density: Density
  transparency: TransparencyPreference
  accent: AccentPreference
}

export const DEFAULT_PREFERENCES: DisplayPreferences = {
  theme: 'system',
  density: 'default',
  transparency: 'system',
  accent: 'system',
}

export function resolveTheme(appearance: Appearance, preferences: DisplayPreferences): ThemeName {
  return preferences.theme === 'system' ? appearance.theme : preferences.theme
}

/**
 * The accent to draw with, as a hex value, or null when there is none to use.
 *
 * Takes the resolved theme rather than working it out again, because a pinned accent has a
 * light and a dark member and picking the wrong one is the difference between a mint that
 * reads and one that does not. Null means "nothing pinned, and Windows would not tell us
 * either", which leaves semantic.css to fall back to its own blue — so the fallback lives in
 * exactly one place rather than being spelled out again here.
 */
export function resolveAccent(
  appearance: Appearance,
  preferences: DisplayPreferences,
  theme: ThemeName,
): string | null {
  if (preferences.accent === 'system') return appearance.accent
  return ACCENT_PALETTE[preferences.accent][theme]
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
/**
 * A `#RRGGBB` string as the `0x00RRGGBB` number the badge is painted with.
 *
 * Returns null for anything that will not parse, so a malformed accent from the OS draws the
 * badge in its fallback rather than in whatever a partial parse happened to produce.
 */
export function rgbNumber(hex: string): number | null {
  const parsed = parseHexColour(hex)
  if (parsed === null) return null

  const [red, green, blue] = parsed
  return (red << 16) | (green << 8) | blue
}

export function applyAppearance(
  appearance: Appearance,
  preferences: DisplayPreferences,
  root: HTMLElement,
): void {
  const theme = resolveTheme(appearance, preferences)

  root.dataset.theme = theme
  root.dataset.density = preferences.density
  root.dataset.backdrop = appearance.backdrop

  if (resolveReduceTransparency(appearance, preferences)) {
    root.dataset.reduceTransparency = ''
  } else {
    delete root.dataset.reduceTransparency
  }

  // Written as inline custom properties rather than as a [data-accent] rule in semantic.css,
  // which was the obvious alternative and is a trap: [data-window-inactive] remaps --accent
  // near the end of that file at the same specificity, so an accent rule placed after it —
  // the natural reading of "it has to come last" — would silently kill the inactive-window
  // desaturation app-wide, with nothing to catch it.
  //
  // Leaving them unset is meaningful: semantic.css falls back to the Apple blue pair, so the
  // fallback value is never duplicated in two places.
  const accent = resolveAccent(appearance, preferences, theme)

  if (accent) {
    root.style.setProperty('--accent-system', accent)
    root.style.setProperty('--accent-fg-system', accentForeground(accent))
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
