import { useState } from 'react'
import { AlertCircle, Check, Minus } from 'lucide-react'

import type { CheckStep } from '@/lib/generated/CheckStep'
import type { DiagnosticReport } from '@/lib/generated/DiagnosticReport'
import { cx } from '@/lib/cx'

import styles from './DiagnosticList.module.css'

/**
 * The connection test's report. docs/04 Phase 4 — *readable, not "authentication failed"*.
 *
 * Three things this does that a plain error string cannot:
 *
 * * Every step is listed, including the ones that never ran. A report that stops halfway
 *   looks like the app gave up; a greyed "Not attempted" says which stage the failure was
 *   at, which is half the diagnosis.
 * * The remedy is the prominent text and the server's own words are folded away. Users act
 *   on the remedy; the raw response only matters when someone has to forward it to an
 *   administrator.
 * * Nothing here is a link to a support page. The remedy says what to do.
 */

function StepIcon({ status }: { status: CheckStep['status'] }) {
  if (status === 'passed') return <Check className={styles.icon} aria-hidden />
  if (status === 'failed') return <AlertCircle className={styles.icon} aria-hidden />
  return <Minus className={styles.icon} aria-hidden />
}

function StepRow({ step }: { step: CheckStep }) {
  const [showRaw, setShowRaw] = useState(false)

  return (
    <li className={styles.row} data-status={step.status}>
      <div className={styles.head}>
        <StepIcon status={step.status} />
        <span className={styles.name}>{step.name}</span>
        <span className="srOnly">
          {step.status === 'passed'
            ? 'passed'
            : step.status === 'failed'
              ? 'failed'
              : 'not attempted'}
        </span>
        {step.status === 'passed' && step.elapsedMs > 0 && (
          <span className={styles.elapsed}>{step.elapsedMs} ms</span>
        )}
      </div>

      <p className={styles.detail}>{step.detail}</p>

      {step.remedy !== null && <p className={styles.remedy}>{step.remedy}</p>}

      {step.serverSaid !== null && step.serverSaid !== '' && (
        <div className={styles.raw}>
          <button
            type="button"
            className={styles.rawToggle}
            aria-expanded={showRaw}
            onClick={() => {
              setShowRaw((open) => !open)
            }}
          >
            {showRaw ? 'Hide what the server said' : 'Show what the server said'}
          </button>

          {showRaw && <pre className={styles.rawBody}>{step.serverSaid}</pre>}
        </div>
      )}
    </li>
  )
}

export interface DiagnosticListProps {
  report: DiagnosticReport
  className?: string | undefined
}

export function DiagnosticList({ report, className }: DiagnosticListProps) {
  return (
    <div className={cx(styles.wrap, className)}>
      <p className={styles.summary} data-ok={report.ok} role="status">
        {report.summary}
      </p>

      <section className={styles.section}>
        <h3 className={styles.heading}>Incoming mail</h3>
        <ul className={styles.list}>
          {report.imap.map((step) => (
            <StepRow key={`imap-${step.name}`} step={step} />
          ))}
        </ul>
      </section>

      <section className={styles.section}>
        <h3 className={styles.heading}>Outgoing mail</h3>
        <ul className={styles.list}>
          {report.smtp.map((step) => (
            <StepRow key={`smtp-${step.name}`} step={step} />
          ))}
        </ul>
      </section>
    </div>
  )
}
