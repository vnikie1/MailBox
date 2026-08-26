import { useAppearanceSync } from '@/app/useAppearanceSync'
import type { Density, ThemePreference } from '@/lib/appearance'
import { useSettingsStore } from '@/store/settings'
import { Button, ScrollArea, ToastProvider } from '@/ui'

import { Specimens } from './Specimens'

import styles from './Gallery.module.css'

const THEMES: { value: ThemePreference; label: string }[] = [
  { value: 'system', label: 'System' },
  { value: 'light', label: 'Light' },
  { value: 'dark', label: 'Dark' },
]

const DENSITIES: { value: Density; label: string }[] = [
  { value: 'compact', label: 'Compact' },
  { value: 'default', label: 'Default' },
  { value: 'comfortable', label: 'Comfortable' },
]

/**
 * The component gallery. Phase 1 exit gate.
 *
 * Every primitive is rendered twice, in two columns forced to `[data-theme='light']` and
 * `[data-theme='dark']`. That works because semantic.css scopes its theme blocks to the
 * attribute rather than to `:root`, so a subtree can carry a different theme from the
 * document — which is the whole point of the token layer, demonstrated.
 *
 * One honest limitation: menus, popovers, tooltips, sheets and toasts render through a
 * portal on `document.body`, so they take the *page* theme rather than their column's.
 * The header's theme control is what shows those in both. Fixing it would mean giving
 * every floating primitive a portal-root prop that only the gallery would ever pass, and
 * standing rule 18 is against shipping scaffolding for a dev tool's benefit.
 *
 * This route is dev-only. main.tsx loads it behind `import.meta.env.DEV` via a dynamic
 * import, so it is not in the production bundle at all.
 */
export function Gallery() {
  useAppearanceSync()

  const theme = useSettingsStore((state) => state.theme)
  const density = useSettingsStore((state) => state.density)
  const setTheme = useSettingsStore((state) => state.setTheme)
  const setDensity = useSettingsStore((state) => state.setDensity)

  return (
    <ToastProvider>
      <div className={styles.gallery}>
        <header className={styles.header}>
          <h1 className={styles.title}>Halcyon — component gallery</h1>

          <div className={styles.controls}>
            <div className={styles.controlGroup} role="group" aria-label="Theme">
              {THEMES.map((option) => (
                <Button
                  key={option.value}
                  aria-pressed={theme === option.value}
                  onClick={() => {
                    setTheme(option.value)
                  }}
                >
                  {option.label}
                </Button>
              ))}
            </div>

            <div className={styles.controlGroup} role="group" aria-label="Density">
              {DENSITIES.map((option) => (
                <Button
                  key={option.value}
                  aria-pressed={density === option.value}
                  onClick={() => {
                    setDensity(option.value)
                  }}
                >
                  {option.label}
                </Button>
              ))}
            </div>
          </div>
        </header>

        {/* The scroll container is the gallery's, not the document's, so the visual
            baseline test has to find it to size the viewport to its full content. */}
        <ScrollArea className={styles.body} data-gallery-scroll="">
          <div className={styles.columns}>
            {/* Landmarks, not plain divs: it is how a test — and a screen-reader user —
                tells the two copies of every specimen apart. */}
            <section className={styles.column} data-theme="light" aria-label="Light">
              <h2 className={styles.columnTitle}>Light</h2>
              <Specimens />
            </section>
            <section className={styles.column} data-theme="dark" aria-label="Dark">
              <h2 className={styles.columnTitle}>Dark</h2>
              <Specimens />
            </section>
          </div>
        </ScrollArea>
      </div>
    </ToastProvider>
  )
}
