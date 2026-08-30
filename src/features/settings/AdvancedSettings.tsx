import { useCallback, useEffect, useState } from 'react'
import { FolderOpen } from 'lucide-react'

import type { CrashReport } from '@/lib/generated/CrashReport'
import {
  crashReportRead,
  crashReports,
  crashReportsClear,
  diagnosticsReveal,
  runningInTauri,
} from '@/lib/ipc'
import { Button, useToast } from '@/ui'

import styles from './settings.module.css'
import pane from './AdvancedSettings.module.css'

/**
 * Settings → Advanced. docs/06 Phase 11.
 *
 * The other end of `diagnostics.rs`. A crash report that is written and never surfaced is a
 * file in a folder nobody knows the name of — the whole point of capturing one is that a person
 * can get hold of it and hand it to somebody who can read it.
 *
 * **Nothing here sends anything.** Reveal opens Explorer; the user decides who sees the file
 * after that. docs/06 asks for "opt-in upload", and there is no destination to upload to — see
 * the note at the top of `diagnostics.rs` for why shipping a client that posts to a server
 * nobody has chosen would be worse than shipping none.
 */

function formatSize(bytes: number): string {
  if (bytes < 1024) return `${String(bytes)} bytes`
  return `${String(Math.round(bytes / 1024))} KB`
}

export function AdvancedSettings() {
  const [reports, setReports] = useState<CrashReport[] | null>(null)
  const [open, setOpen] = useState<string | null>(null)
  const [text, setText] = useState<string>('')
  const toast = useToast()

  const refresh = useCallback(() => {
    void crashReports().then(setReports)
  }, [])

  useEffect(refresh, [refresh])

  const show = (report: CrashReport) => {
    if (open === report.name) {
      setOpen(null)
      return
    }

    setOpen(report.name)
    setText('')
    void crashReportRead(report.name).then(setText)
  }

  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>Diagnostics</h3>

      <p className={styles.hint}>
        Halcyon keeps a log and, if it ever stops unexpectedly, a report of what it was doing. Both
        stay on this machine. Nothing is uploaded.
      </p>

      <div className={styles.row}>
        <Button
          variant="bordered"
          onClick={() => {
            void diagnosticsReveal()
          }}
        >
          <FolderOpen size={16} aria-hidden />
          Open the diagnostics folder
        </Button>

        {reports !== null && reports.length > 0 && (
          <Button
            variant="bordered"
            onClick={() => {
              void crashReportsClear().then((removed) => {
                toast.show({
                  title: removed === 1 ? '1 report deleted' : `${String(removed)} reports deleted`,
                })
                setOpen(null)
                refresh()
              })
            }}
          >
            Delete all reports
          </Button>
        )}
      </div>

      <h4 className={styles.legend}>Crash reports</h4>

      {reports === null ? (
        <p className={styles.hint}>Looking…</p>
      ) : reports.length === 0 ? (
        <p className={styles.hint}>
          {runningInTauri
            ? 'None. Halcyon has not crashed on this machine.'
            : 'Crash reports are written by the app, so there are none in a browser.'}
        </p>
      ) : (
        <ul className={pane.reports}>
          {reports.map((report) => (
            <li key={report.name} className={pane.report}>
              <button
                type="button"
                className={pane.rowButton}
                aria-expanded={open === report.name}
                onClick={() => {
                  show(report)
                }}
              >
                <span className={pane.summary}>{report.summary}</span>
                <span className={styles.name}>{formatSize(report.bytes)}</span>
              </button>

              {open === report.name && (
                <pre className={pane.detail}>{text === '' ? 'Reading…' : text}</pre>
              )}
            </li>
          ))}
        </ul>
      )}
    </section>
  )
}
