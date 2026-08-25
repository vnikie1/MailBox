import { accentForeground } from '@/lib/appearance'
import { runningInTauri } from '@/lib/ipc'
import { useAppearanceStore } from '@/store/appearance'

import styles from './ShellDiagnostics.module.css'

/**
 * What the shell currently believes about its host.
 *
 * This is Phase 0's content, and it is the surface the Phase 0 exit gate is read from:
 * it shows, live, that the theme follows Windows, that the accent is the OS accent, which
 * DWM material actually took effect, and whether the window is active. Phase 2 replaces
 * it with the three panes.
 */
export function ShellDiagnostics() {
  const appearance = useAppearanceStore((state) => state.appearance)
  const windowActive = useAppearanceStore((state) => state.windowActive)

  const backdropLabel: Record<typeof appearance.backdrop, string> = {
    micaAlt: 'Mica Alt (samples the wallpaper)',
    none: 'none — opaque surfaces',
  }

  return (
    <section className={styles.panel}>
      <h1 className={styles.title}>Halyard</h1>
      <p className={styles.subtitle}>
        Phase 0 — window shell. Theme, accent and backdrop below are read from Windows and pushed by
        the core; nothing here polls.
      </p>

      <dl className={styles.grid}>
        <dt className={styles.term}>Host</dt>
        <dd className={styles.value}>
          {runningInTauri ? 'Tauri WebView2' : 'browser preview (no Win32 layer)'}
        </dd>

        <dt className={styles.term}>Theme</dt>
        <dd className={styles.value}>{appearance.theme}</dd>

        <dt className={styles.term}>OS accent</dt>
        <dd className={styles.value}>
          {appearance.accent ? (
            <>
              <span
                className={styles.swatch}
                style={{
                  backgroundColor: appearance.accent,
                  color: accentForeground(appearance.accent),
                }}
              >
                Aa
              </span>
              <span className="tabular">{appearance.accent}</span>
            </>
          ) : (
            'not reported — using the Apple blue fallback'
          )}
        </dd>

        <dt className={styles.term}>Backdrop</dt>
        <dd className={styles.value}>{backdropLabel[appearance.backdrop]}</dd>

        <dt className={styles.term}>Reduce transparency</dt>
        <dd className={styles.value}>{appearance.reduceTransparency ? 'on' : 'off'}</dd>

        <dt className={styles.term}>Window</dt>
        <dd className={styles.value}>{windowActive ? 'active' : 'inactive'}</dd>

        <dt className={styles.term}>Display scaling</dt>
        <dd className={`${styles.value} tabular`}>{Math.round(window.devicePixelRatio * 100)}%</dd>
      </dl>
    </section>
  )
}
