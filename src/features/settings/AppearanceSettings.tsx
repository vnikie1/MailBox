import {
  ACCENT_NAMES,
  ACCENT_PALETTE,
  resolveTheme,
  type AccentPreference,
  type Density,
  type ThemePreference,
  type TransparencyPreference,
} from '@/lib/appearance'
import { cx } from '@/lib/cx'
import { useAppearanceStore } from '@/store/appearance'
import { useSettingsStore } from '@/store/settings'
import { Segmented, Select, type SegmentedOption, type SelectOption } from '@/ui'

import { Field, Form } from './SettingsForm'
import styles from './settings.module.css'

/**
 * Theme, density and transparency. docs/02 §5, docs/06 Phase 11.
 *
 * These three have existed since Phase 1 — the store, the resolution against what Windows
 * reports, the pre-paint application in `main.tsx`, the token remaps they drive. What they have
 * never had is a control. Until now the only way to change the density of the message list was
 * to edit localStorage by hand, which means that for ten phases the app has had a design system
 * with a Reduce Transparency escape hatch that nobody could reach — and docs/02 §5 asks for that
 * escape hatch specifically for users whose GPU makes the Mica backdrop painful.
 *
 * Every option here has a **System** setting and it is the default, because Windows already
 * knows the answer for two of the three and asking again is how an app ends up in dark mode
 * when the desktop is light.
 *
 * ## Why two different controls
 *
 * Theme and density are segmented controls; translucency is a popup. The rule is whether seeing
 * the alternatives is worth the width: Light against Dark is a comparison, and three words of
 * "Always translucent / Never translucent / Follow Windows" is a paragraph laid sideways. This
 * section used to be four stacks of radios — twelve rows, four legends, and a pane that
 * scrolled to hold four answers.
 */

const THEMES: SegmentedOption<ThemePreference>[] = [
  { value: 'system', label: 'Follow Windows' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
]

const DENSITIES: (SegmentedOption<Density> & { hint: string })[] = [
  { value: 'compact', label: 'Compact', hint: 'More messages on screen.' },
  // Not "What Mail uses." — that named the app this one is modelled on, in the settings window
  // of the app the reader is actually running, where "Mail" is at best the Windows app of that
  // name. A hint has to describe the effect, not the provenance.
  { value: 'default', label: 'Default', hint: 'The standard row height.' },
  { value: 'comfortable', label: 'Comfortable', hint: 'Larger text and taller rows.' },
]

const TRANSPARENCIES: SelectOption<TransparencyPreference>[] = [
  { value: 'system', label: 'Follow Windows' },
  { value: 'full', label: 'Always translucent' },
  { value: 'reduce', label: 'Never translucent' },
]

/**
 * The names shown beside each swatch.
 *
 * A swatch grid without names is a control only a sighted user can operate, and "the fourth
 * circle" is not something a screen reader can say. These are also what the labels announce,
 * so they have to read as colour names rather than as token ids.
 */
const ACCENT_LABELS: Record<string, string> = {
  blue: 'Blue',
  red: 'Red',
  orange: 'Orange',
  yellow: 'Yellow',
  green: 'Green',
  mint: 'Mint',
  teal: 'Teal',
  indigo: 'Indigo',
  purple: 'Purple',
  pink: 'Pink',
  gray: 'Graphite',
}

export function AppearanceSettings() {
  const theme = useSettingsStore((state) => state.theme)
  const density = useSettingsStore((state) => state.density)
  const transparency = useSettingsStore((state) => state.transparency)

  const accent = useSettingsStore((state) => state.accent)

  const setTheme = useSettingsStore((state) => state.setTheme)
  const setDensity = useSettingsStore((state) => state.setDensity)
  const setTransparency = useSettingsStore((state) => state.setTransparency)
  const setAccent = useSettingsStore((state) => state.setAccent)

  // The swatches preview the colour the app would actually draw, which means resolving the
  // theme here too: every hue has a light and a dark member, and showing the light one while
  // the app is dark would make the picker lie about its own result.
  const appearance = useAppearanceStore((state) => state.appearance)
  const resolvedTheme = resolveTheme(appearance, {
    theme,
    density,
    transparency,
    accent,
  })

  // "Follow Windows" first and selected by default, for the same reason every other setting
  // on this pane leads with it: Windows already knows the answer, and asking again is how an
  // app ends up the one thing on the desktop wearing a different colour.
  const ACCENT_OPTIONS: { id: AccentPreference; label: string; preview: string | null }[] = [
    {
      id: 'system',
      label: 'Follow Windows',
      // The OS accent when Windows reports one — and the colour the app would *actually*
      // draw when it does not, which is the same fallback semantic.css uses. Leaving this
      // null drew an empty circle: no fill, the pane showing through, a swatch that looks
      // like a rendering fault rather than a choice. A machine that reports no accent is not
      // an error state, and the picker should show what it would get.
      preview: appearance.accent ?? ACCENT_PALETTE.blue[resolvedTheme],
    },
    ...ACCENT_NAMES.map((name) => ({
      id: name,
      label: ACCENT_LABELS[name] ?? name,
      preview: ACCENT_PALETTE[name][resolvedTheme],
    })),
  ]

  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Appearance</h2>

      <Form>
        <Field label="Theme" labelId="theme-label">
          <Segmented
            label="Theme"
            labelledBy="theme-label"
            options={THEMES}
            value={theme}
            onValueChange={setTheme}
          />
        </Field>

        <Field
          label="Accent colour"
          labelId="accent-label"
          hint={
            accent === 'system'
              ? 'Following the accent colour Windows is set to.'
              : `Using ${ACCENT_LABELS[accent] ?? accent} instead of the Windows accent colour.`
          }
        >
          <div className={styles.swatches} role="radiogroup" aria-labelledby="accent-label">
            {ACCENT_OPTIONS.map((option) => (
              <label
                key={option.id}
                className={cx(styles.swatch, option.id === 'system' && styles.swatchSystem)}
                title={option.label}
              >
                <input
                  type="radio"
                  name="accent"
                  className={styles.swatchInput}
                  aria-label={option.label}
                  checked={accent === option.id}
                  onChange={() => {
                    setAccent(option.id)
                  }}
                />
                {/*
                  The colour arrives as an inline style rather than from a token, and that is
                  deliberate. Standing rule 1 keeps raw colour out of components because a
                  component should not invent one — but this is not the component choosing a
                  colour, it is rendering a value the user is picking from, and the values are
                  the same eleven pairs primitive.css holds. tests/unit/accentPalette.test.ts
                  fails if the two ever disagree.
                */}
                <span
                  className={styles.swatchFill}
                  style={option.preview === null ? undefined : { background: option.preview }}
                  aria-hidden="true"
                />
              </label>
            ))}
          </div>
        </Field>

        <Field
          label="Density"
          labelId="density-label"
          hint={DENSITIES.find((choice) => choice.value === density)?.hint}
        >
          <Segmented
            label="Density"
            labelledBy="density-label"
            options={DENSITIES}
            value={density}
            onValueChange={setDensity}
          />
        </Field>

        <Field
          label="Translucency"
          htmlFor="translucency"
          hint="The sidebar and toolbar pick up a tint of whatever is behind the window. Turning it off costs nothing in appearance terms and can help where the effect is expensive to draw."
        >
          <Select
            id="translucency"
            label="Translucency"
            hideLabel
            options={TRANSPARENCIES}
            value={transparency}
            onValueChange={setTransparency}
          />
        </Field>
      </Form>
    </section>
  )
}
