import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { describe, expect, it } from 'vitest'

import { DiagnosticList } from '@/features/accounts/DiagnosticList'
import type { CheckStep } from '@/lib/generated/CheckStep'
import type { DiagnosticReport } from '@/lib/generated/DiagnosticReport'

function step(overrides: Partial<CheckStep> & { name: string }): CheckStep {
  return {
    status: 'passed',
    detail: 'Fine.',
    remedy: null,
    serverSaid: null,
    elapsedMs: 0,
    ...overrides,
  }
}

/** The Microsoft failure docs/05 §3 singles out, as the report renders it. */
const smtpAuthDisabled: DiagnosticReport = {
  ok: false,
  imap: [
    step({ name: 'Connect', detail: 'Reached outlook.office365.com:993.', elapsedMs: 42 }),
    step({ name: 'Sign in', detail: 'Signed in successfully.' }),
  ],
  smtp: [
    step({ name: 'Connect', detail: 'Reached smtp.office365.com:587.' }),
    step({
      name: 'Sign in',
      status: 'failed',
      detail: 'The outgoing server rejected the sign-in.',
      remedy:
        'Your organisation has turned off SMTP authentication for this mailbox. An administrator has to enable SMTP AUTH for it — your password is not the problem.',
      serverSaid:
        '535 5.7.139 Authentication unsuccessful, SmtpClientAuthentication is disabled for the Tenant.',
    }),
  ],
  summary:
    'Your organisation has turned off SMTP authentication for this mailbox. An administrator has to enable SMTP AUTH for it — your password is not the problem.',
}

describe('the diagnostic report', () => {
  it('leads with the remedy rather than the protocol error', () => {
    // The whole point of docs/04 Phase 4's "readable diagnostic report, not authentication
    // failed". The first thing on screen has to be the thing the user can act on.
    render(<DiagnosticList report={smtpAuthDisabled} />)

    const summary = screen.getByRole('status')
    expect(summary.textContent).toContain('administrator')
    expect(summary.textContent).toContain('password is not the problem')
    expect(summary.textContent).not.toContain('5.7.139')
  })

  it('separates incoming from outgoing, because they fail independently', () => {
    render(<DiagnosticList report={smtpAuthDisabled} />)

    expect(screen.getByRole('heading', { name: 'Incoming mail' })).toBeInTheDocument()
    expect(screen.getByRole('heading', { name: 'Outgoing mail' })).toBeInTheDocument()
  })

  it('hides the raw server response until it is asked for', () => {
    // Useless to most users, and the only thing that helps when someone has to forward it
    // to an administrator.
    render(<DiagnosticList report={smtpAuthDisabled} />)

    expect(screen.queryByText(/5\.7\.139/)).not.toBeInTheDocument()
    expect(screen.getByRole('button', { name: /show what the server said/i })).toBeInTheDocument()
  })

  it('reveals the raw response on request', async () => {
    const user = userEvent.setup()
    render(<DiagnosticList report={smtpAuthDisabled} />)

    await user.click(screen.getByRole('button', { name: /show what the server said/i }))

    expect(screen.getByText(/5\.7\.139/)).toBeInTheDocument()
    expect(screen.getByRole('button', { name: /hide what the server said/i })).toHaveAttribute(
      'aria-expanded',
      'true',
    )
  })

  it('lists the steps that never ran instead of stopping halfway', () => {
    // A report that stops at the failure looks like the app gave up. Which stage was not
    // reached is half the diagnosis.
    const report: DiagnosticReport = {
      ok: false,
      imap: [
        step({
          name: 'Connect',
          status: 'failed',
          detail: 'Could not reach imap.example.test:993.',
          remedy: 'Check the server name and that a firewall is not blocking the connection.',
        }),
        step({ name: 'Secure the connection', status: 'skipped', detail: 'Not attempted.' }),
        step({ name: 'Sign in', status: 'skipped', detail: 'Not attempted.' }),
      ],
      smtp: [],
      summary: 'Check the server name and that a firewall is not blocking the connection.',
    }

    render(<DiagnosticList report={report} />)

    expect(screen.getByText('Secure the connection')).toBeInTheDocument()
    expect(screen.getAllByText('Not attempted.')).toHaveLength(2)
  })

  it('announces pass and fail in text, not only in colour', () => {
    // docs/02 §8: colour is never the only carrier of meaning, and a green tick beside a
    // red cross is exactly the case where that matters.
    render(<DiagnosticList report={smtpAuthDisabled} />)

    expect(screen.getAllByText('passed').length).toBeGreaterThan(0)
    expect(screen.getAllByText('failed')).toHaveLength(1)
  })

  it('shows no remedy on a step that passed', () => {
    const report: DiagnosticReport = {
      ok: true,
      imap: [step({ name: 'Sign in', detail: 'Signed in successfully.', elapsedMs: 120 })],
      smtp: [step({ name: 'Sign in', detail: 'Signed in to the outgoing server.' })],
      summary: 'Halcyon connected to both servers and signed in successfully.',
    }

    render(<DiagnosticList report={report} />)

    expect(screen.getByRole('status').textContent).toContain('successfully')
    expect(screen.queryByRole('button', { name: /server said/i })).not.toBeInTheDocument()
    expect(screen.getByText('120 ms')).toBeInTheDocument()
  })
})
