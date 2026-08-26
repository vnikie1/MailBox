import { describe, expect, it } from 'vitest'

import {
  INITIAL,
  canContinue,
  looksLikeEmail,
  reduce,
  titleFor,
  type AssistantState,
} from '@/features/accounts/model'
import type { DiagnosticReport } from '@/lib/generated/DiagnosticReport'
import type { ProviderInfo } from '@/lib/generated/ProviderInfo'

const google: ProviderInfo = {
  id: 'google',
  displayName: 'Google',
  authKind: 'oAuth2',
  needsManualSetup: false,
  setupNote: 'Sign in happens in your browser.',
  setupUrl: null,
  needsOauthClient: false,
  requiresClientSecret: true,
}

const icloud: ProviderInfo = {
  id: 'icloud',
  displayName: 'iCloud',
  authKind: 'password',
  needsManualSetup: false,
  setupNote: 'iCloud needs an app-specific password.',
  setupUrl: 'https://appleid.apple.com/account/manage',
  needsOauthClient: false,
  requiresClientSecret: false,
}

const other: ProviderInfo = {
  id: 'other',
  displayName: 'Other Mail Account',
  authKind: 'password',
  needsManualSetup: true,
  setupNote: null,
  setupUrl: null,
  needsOauthClient: false,
  requiresClientSecret: false,
}

function report(ok: boolean): DiagnosticReport {
  return {
    ok,
    imap: [
      {
        name: 'Sign in',
        status: ok ? 'passed' : 'failed',
        detail: ok ? 'Signed in.' : 'Rejected.',
        remedy: ok ? null : 'Check the password.',
        serverSaid: null,
        elapsedMs: 4,
      },
    ],
    smtp: [],
    summary: ok ? 'Connected.' : 'Check the password.',
  }
}

/** Walks the flow up to a filled-in identity step for the given provider. */
function atIdentity(provider: ProviderInfo, password = 'app-specific-pw'): AssistantState {
  let state = reduce(INITIAL, { type: 'pickProvider', provider })
  state = reduce(state, { type: 'continue' })
  state = reduce(state, { type: 'edit', field: 'email', value: 'ada@example.test' })
  if (provider.authKind === 'password') {
    state = reduce(state, { type: 'edit', field: 'password', value: password })
  }
  return state
}

describe('email shape', () => {
  it('accepts the addresses people actually have', () => {
    expect(looksLikeEmail('ada@example.test')).toBe(true)
    expect(looksLikeEmail('ada.lovelace+mail@sub.example.co.uk')).toBe(true)
    // A quoted local part may contain an @; splitting on the first would break it.
    expect(looksLikeEmail('"a@b"@example.test')).toBe(true)
  })

  it('rejects what cannot be a mail domain', () => {
    expect(looksLikeEmail('ada')).toBe(false)
    expect(looksLikeEmail('ada@')).toBe(false)
    expect(looksLikeEmail('@example.test')).toBe(false)
    // No dot means no domain to look up, and probing "localhost" would reach a machine the
    // user never named.
    expect(looksLikeEmail('ada@localhost')).toBe(false)
  })
})

describe('the assistant flow', () => {
  it('starts on the provider step and cannot continue without a choice', () => {
    expect(INITIAL.step).toBe('provider')
    expect(canContinue(INITIAL)).toBe(false)
  })

  it('refuses to continue with a provider whose sign-in application is not set up', () => {
    // Otherwise the browser opens onto a Google error page, which reads as the app being
    // broken rather than unconfigured.
    const unconfigured = { ...google, needsOauthClient: true }
    const state = reduce(INITIAL, { type: 'pickProvider', provider: unconfigured })

    expect(canContinue(state)).toBe(false)
  })

  it('asks an OAuth account for no password at all', () => {
    // There is nothing for the user to type, and a password box would invite them to type
    // their Google password into an app that must never see it.
    const state = atIdentity(google)

    expect(state.password).toBe('')
    expect(canContinue(state)).toBe(true)
  })

  it('will not continue past identity without a password for a password account', () => {
    let state = reduce(INITIAL, { type: 'pickProvider', provider: icloud })
    state = reduce(state, { type: 'continue' })
    state = reduce(state, { type: 'edit', field: 'email', value: 'ada@icloud.test' })

    expect(canContinue(state)).toBe(false)

    state = reduce(state, { type: 'edit', field: 'password', value: 'abcd-efgh-ijkl-mnop' })
    expect(canContinue(state)).toBe(true)
  })

  it('skips the server form when the servers are already known', () => {
    // Asking a Gmail user for an IMAP hostname the app already has is the thing the
    // provider picker exists to avoid.
    const state = reduce(atIdentity(icloud), { type: 'continue' })

    expect(state.step).toBe('testing')
  })

  it('shows the server form for a provider whose servers are unknown', () => {
    const state = reduce(atIdentity(other), { type: 'continue' })

    expect(state.step).toBe('servers')
    // Prefilled with the conventional ports rather than left blank.
    expect(state.imap?.port).toBe(993)
    expect(state.smtp?.port).toBe(587)
    expect(canContinue(state)).toBe(false)
  })

  it('shows the server form when the servers were only guessed', () => {
    // A probed result is a guess with a TCP handshake behind it. The user should see it
    // before the app tries to sign in with it.
    let state = atIdentity(icloud)
    state = reduce(state, {
      type: 'discovered',
      discovery: {
        imap: { host: 'imap.example.test', port: 993, security: 'tls' },
        smtp: { host: 'smtp.example.test', port: 587, security: 'startTls' },
        source: 'probe',
        explanation: 'Guessed from the domain name.',
        needsConfirmation: true,
        suggestedProvider: null,
      },
    })

    expect(reduce(state, { type: 'continue' }).step).toBe('servers')
    // And prefilled from what was found, in the form the core takes back.
    expect(state.imap?.host).toBe('imap.example.test')
    expect(state.smtp?.security).toBe('starttls')
  })

  it('switches provider when the domain turns out to be hosted by one', () => {
    // A custom domain on Google rejects passwords. Leaving the user on a password form
    // produces a failure with no explanation.
    let state = atIdentity(other)
    state = reduce(state, {
      type: 'discovered',
      provider: google,
      discovery: {
        imap: { host: 'imap.gmail.com', port: 993, security: 'tls' },
        smtp: { host: 'smtp.gmail.com', port: 587, security: 'startTls' },
        source: 'autoconfig',
        explanation: 'Published by the mail domain itself.',
        needsConfirmation: false,
        suggestedProvider: 'google',
      },
    })

    expect(state.provider?.id).toBe('google')
    expect(reduce(state, { type: 'continue' }).step).toBe('testing')
  })

  it('clears server fields when the provider changes', () => {
    // Carrying imap.gmail.com into a Yahoo account is a confusing failure that looks like
    // the app being wrong about the user's own provider.
    let state = reduce(atIdentity(other), {
      type: 'editServer',
      which: 'imap',
      patch: { host: 'imap.old.test' },
    })
    state = reduce(state, { type: 'pickProvider', provider: icloud })

    expect(state.imap).toBeNull()
    expect(state.discovery).toBeNull()
  })
})

describe('the report step', () => {
  it('goes back to the fields on a failure, keeping the password', () => {
    // Retyping a 16-character app-specific password after a server-name typo is the kind
    // of small cruelty that makes people give up on an app.
    const tested = reduce(atIdentity(icloud), { type: 'tested', report: report(false) })
    expect(tested.step).toBe('report')

    const retry = reduce(tested, { type: 'continue' })
    expect(retry.step).toBe('identity')
    expect(retry.password).toBe('app-specific-pw')
    expect(retry.report).toBeNull()
  })

  it('moves on only when the test passed', () => {
    const tested = reduce(atIdentity(icloud), { type: 'tested', report: report(true) })

    expect(reduce(tested, { type: 'continue' }).step).toBe('done')
  })

  it('names the outcome in the title rather than saying "Report"', () => {
    const failed = reduce(atIdentity(icloud), { type: 'tested', report: report(false) })
    const passed = reduce(atIdentity(icloud), { type: 'tested', report: report(true) })

    expect(titleFor(failed)).toBe('Could Not Connect')
    expect(titleFor(passed)).toBe('Account Ready')
    expect(titleFor(INITIAL)).toContain('Provider')
  })

  it('puts a failure back on a step the user can act on, never on the spinner', () => {
    // Being stranded on "Checking the Connection" with an error underneath is the worst of
    // both: nothing is happening and there is no way forward.
    const testing = reduce(atIdentity(icloud), { type: 'continue' })
    expect(testing.step).toBe('testing')

    const failed = reduce(testing, { type: 'failed', message: 'The network is unreachable.' })

    expect(failed.step).toBe('identity')
    expect(failed.error).toBe('The network is unreachable.')
    expect(failed.busy).toBe(false)
  })
})

describe('resetting', () => {
  it('drops the password and the report', () => {
    // The sheet resets on close, so reopening it never flashes the last attempt's
    // credential or its diagnostic report.
    const used = reduce(atIdentity(icloud), { type: 'tested', report: report(false) })
    const reset = reduce(used, { type: 'reset' })

    expect(reset).toEqual(INITIAL)
    expect(reset.password).toBe('')
    expect(reset.report).toBeNull()
  })
})

describe('busy', () => {
  it('blocks every step while work is in flight', () => {
    // Double-submitting a sign-in opens two browser windows and two loopback listeners.
    const busy = { ...atIdentity(icloud), busy: true }

    expect(canContinue(busy)).toBe(false)
  })
})

describe('titles', () => {
  it('does not say "Other Mail Account Account"', () => {
    // The obvious template — `Add Your ${displayName} Account` — produces exactly that for
    // the one provider whose name already ends in the word. Seen in the running app, not
    // in any test, which is the Phase 2 and 3 lesson again.
    const state = reduce(INITIAL, { type: 'pickProvider', provider: other })

    expect(titleFor(reduce(state, { type: 'continue' }))).toBe('Add a Mail Account')
  })

  it('still names the provider for everyone else', () => {
    const state = reduce(INITIAL, { type: 'pickProvider', provider: icloud })

    expect(titleFor(reduce(state, { type: 'continue' }))).toBe('Add Your iCloud Account')
  })
})
