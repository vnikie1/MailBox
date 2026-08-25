import { ShellDiagnostics } from '@/features/diagnostics/ShellDiagnostics'
import { Titlebar } from '@/features/titlebar/Titlebar'

import { useAppearanceSync } from './useAppearanceSync'

import styles from './App.module.css'

export function App() {
  useAppearanceSync()

  return (
    <div className={styles.window}>
      <Titlebar />
      <div className={styles.body}>
        <div className={styles.sidebar} />
        <main className={styles.content}>
          <ShellDiagnostics />
        </main>
      </div>
    </div>
  )
}
