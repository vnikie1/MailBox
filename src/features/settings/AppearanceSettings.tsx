import type { Density, ThemePreference, TransparencyPreference } from '@/lib/appearance'
import { useSettingsStore } from '@/store/settings'

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
 */

const THEMES: { id: ThemePreference; label: string }[] = [
  { id: 'system', label: 'Follow Windows' },
  { id: 'light', label: 'Light' },
  { id: 'dark', label: 'Dark' },
]

const DENSITIES: { id: Density; label: string; hint: string }[] = [
  { id: 'compact', label: 'Compact', hint: 'More messages on screen.' },
  { id: 'default', label: 'Default', hint: 'What Mail uses.' },
  { id: 'comfortable', label: 'Comfortable', hint: 'Larger text and taller rows.' },
]

const TRANSPARENCIES: { id: TransparencyPreference; label: string }[] = [
  { id: 'system', label: 'Follow Windows' },
  { id: 'full', label: 'Always translucent' },
  { id: 'reduce', label: 'Never translucent' },
]

export function AppearanceSettings() {
  const theme = useSettingsStore((state) => state.theme)
  const density = useSettingsStore((state) => state.density)
  const transparency = useSettingsStore((state) => state.transparency)

  const setTheme = useSettingsStore((state) => state.setTheme)
  const setDensity = useSettingsStore((state) => state.setDensity)
  const setTransparency = useSettingsStore((state) => state.setTransparency)

  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>Appearance</h3>

      <fieldset className={styles.group}>
        <legend className={styles.legend}>Theme</legend>

        {THEMES.map((choice) => (
          <label key={choice.id} className={styles.choice}>
            <input
              type="radio"
              name="theme"
              className={styles.radio}
              checked={theme === choice.id}
              onChange={() => {
                setTheme(choice.id)
              }}
            />
            {choice.label}
          </label>
        ))}
      </fieldset>

      <fieldset className={styles.group}>
        <legend className={styles.legend}>Density</legend>

        {DENSITIES.map((choice) => (
          <label key={choice.id} className={styles.choice}>
            <input
              type="radio"
              name="density"
              className={styles.radio}
              checked={density === choice.id}
              onChange={() => {
                setDensity(choice.id)
              }}
            />
            {choice.label}
          </label>
        ))}
      </fieldset>

      <p className={styles.hint}>{DENSITIES.find((choice) => choice.id === density)?.hint}</p>

      <fieldset className={styles.group}>
        <legend className={styles.legend}>Translucency</legend>

        {TRANSPARENCIES.map((choice) => (
          <label key={choice.id} className={styles.choice}>
            <input
              type="radio"
              name="transparency"
              className={styles.radio}
              checked={transparency === choice.id}
              onChange={() => {
                setTransparency(choice.id)
              }}
            />
            {choice.label}
          </label>
        ))}
      </fieldset>

      <p className={styles.hint}>
        The sidebar and the toolbar pick up a tint of whatever is behind the window. Turning it off
        costs nothing in appearance terms and can help on a machine where the effect is expensive to
        draw.
      </p>
    </section>
  )
}
