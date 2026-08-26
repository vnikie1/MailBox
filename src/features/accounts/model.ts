import type { DiagnosticReport } from '@/lib/generated/DiagnosticReport'
import type { DiscoveryResult } from '@/lib/generated/DiscoveryResult'
import type { ProviderInfo } from '@/lib/generated/ProviderInfo'
import type { ServerInput } from '@/lib/generated/ServerInput'

/**
 * The account assistant's flow, as data. docs/04 Phase 4.
 *
 * Kept out of the component because the interesting part is which step follows which, and
 * that depends on the provider, on what autodiscovery found, and on whether the connection
 * test passed. Testing that as a reducer is worth more than testing it through six renders
 * of a modal.
 *
 * The one rule the whole flow exists to enforce: **the connection is tested before the
 * account is saved.** An account row that cannot connect is worse than no row — it appears
 * in the sidebar, fails quietly, and the user has to work out why on their own.
 */

export type StepName = 'provider' | 'identity' | 'servers' | 'testing' | 'report' | 'done'

export interface AssistantState {
  step: StepName
  provider: ProviderInfo | null
  displayName: string
  email: string
  password: string
  /** Present once autodiscovery has answered, or once the user has edited the servers. */
  imap: ServerInput | null
  smtp: ServerInput | null
  discovery: DiscoveryResult | null
  report: DiagnosticReport | null
  /** A message from the core, shown above the footer. Never a secret. */
  error: string | null
  busy: boolean
}

export const INITIAL: AssistantState = {
  step: 'provider',
  provider: null,
  displayName: '',
  email: '',
  password: '',
  imap: null,
  smtp: null,
  discovery: null,
  report: null,
  error: null,
  busy: false,
}

export type Action =
  | { type: 'pickProvider'; provider: ProviderInfo }
  | { type: 'edit'; field: 'displayName' | 'email' | 'password'; value: string }
  | { type: 'editServer'; which: 'imap' | 'smtp'; patch: Partial<ServerInput> }
  | { type: 'discovered'; discovery: DiscoveryResult | null; provider?: ProviderInfo | undefined }
  | { type: 'continue' }
  | { type: 'back' }
  | { type: 'busy'; busy: boolean }
  | { type: 'tested'; report: DiagnosticReport }
  | { type: 'failed'; message: string }
  | { type: 'added' }
  | { type: 'reset' }

const DEFAULT_IMAP: ServerInput = { host: '', port: 993, security: 'tls' }
const DEFAULT_SMTP: ServerInput = { host: '', port: 587, security: 'starttls' }

/** A cheap shape check, not a validator. Anything stricter rejects addresses that work. */
export function looksLikeEmail(value: string): boolean {
  const trimmed = value.trim()
  const at = trimmed.lastIndexOf('@')
  if (at <= 0 || at === trimmed.length - 1) return false

  const domain = trimmed.slice(at + 1)
  return domain.includes('.') && !domain.startsWith('.') && !domain.endsWith('.')
}

/** Whether the current step's fields are filled in enough to move on. */
export function canContinue(state: AssistantState): boolean {
  if (state.busy) return false

  switch (state.step) {
    case 'provider':
      // A provider whose OAuth client is not configured cannot sign anyone in, so the
      // assistant sends the user to set one up rather than letting them walk into a
      // browser error page.
      return state.provider !== null && !state.provider.needsOauthClient

    case 'identity': {
      if (!looksLikeEmail(state.email)) return false
      // OAuth accounts have no password field at all — the browser handles it.
      if (state.provider?.authKind === 'oAuth2') return true
      return state.password.length > 0
    }

    case 'servers':
      return (state.imap?.host.trim() ?? '') !== '' && (state.smtp?.host.trim() ?? '') !== ''

    case 'report':
      return true

    default:
      return false
  }
}

/**
 * Which step follows `identity`.
 *
 * Only an account whose servers are unknown stops at the server form. Showing it to a Gmail
 * user would be asking a question the app already knows the answer to.
 */
function afterIdentity(state: AssistantState): StepName {
  if (state.provider?.needsManualSetup === true) return 'servers'
  if (state.discovery?.needsConfirmation === true) return 'servers'
  return 'testing'
}

export function reduce(state: AssistantState, action: Action): AssistantState {
  switch (action.type) {
    case 'pickProvider':
      return {
        ...state,
        provider: action.provider,
        // Server fields are cleared rather than kept: carrying imap.gmail.com over into a
        // Yahoo account is a confusing failure that looks like the app being wrong.
        imap: action.provider.needsManualSetup ? DEFAULT_IMAP : null,
        smtp: action.provider.needsManualSetup ? DEFAULT_SMTP : null,
        discovery: null,
        report: null,
        error: null,
      }

    case 'edit':
      return { ...state, [action.field]: action.value, error: null }

    case 'editServer': {
      const base =
        action.which === 'imap' ? (state.imap ?? DEFAULT_IMAP) : (state.smtp ?? DEFAULT_SMTP)
      return { ...state, [action.which]: { ...base, ...action.patch }, error: null }
    }

    case 'discovered': {
      const provider = action.provider ?? state.provider

      return {
        ...state,
        provider,
        discovery: action.discovery,
        // Prefilled from the lookup, and still editable — a probed result is a guess, and
        // `needsConfirmation` is what puts the user in front of it.
        imap: action.discovery
          ? {
              ...action.discovery.imap,
              security: action.discovery.imap.security === 'startTls' ? 'starttls' : 'tls',
            }
          : state.imap,
        smtp: action.discovery
          ? {
              ...action.discovery.smtp,
              security: action.discovery.smtp.security === 'startTls' ? 'starttls' : 'tls',
            }
          : state.smtp,
      }
    }

    case 'continue': {
      if (!canContinue(state)) return state

      switch (state.step) {
        case 'provider':
          return { ...state, step: 'identity', error: null }
        case 'identity':
          return { ...state, step: afterIdentity(state), error: null }
        case 'servers':
          return { ...state, step: 'testing', error: null }
        case 'report':
          // A failed report goes back to the fields rather than forward. The password is
          // kept: retyping it after a server-name typo is the kind of small cruelty that
          // makes people give up.
          return state.report?.ok === true
            ? { ...state, step: 'done' }
            : { ...state, step: 'identity', report: null }
        default:
          return state
      }
    }

    case 'back':
      switch (state.step) {
        case 'identity':
          return { ...state, step: 'provider', error: null }
        case 'servers':
          return { ...state, step: 'identity', error: null }
        case 'report':
          return { ...state, step: 'identity', report: null, error: null }
        default:
          return state
      }

    case 'busy':
      return { ...state, busy: action.busy }

    case 'tested':
      return { ...state, step: 'report', report: action.report, busy: false }

    case 'failed':
      // Back to the fields, not stranded on a spinner. The step that failed is the one the
      // user can act on.
      return {
        ...state,
        step: state.step === 'testing' ? 'identity' : state.step,
        error: action.message,
        busy: false,
      }

    case 'added':
      return { ...state, step: 'done', busy: false }

    case 'reset':
      return INITIAL

    default:
      return state
  }
}

/** The heading for each step, so the sheet title and the tests agree on one source. */
export function titleFor(state: AssistantState): string {
  switch (state.step) {
    case 'provider':
      return 'Choose a Mail Account Provider'
    case 'identity':
      if (state.provider === null) return 'Add an Account'
      // "Other Mail Account" already ends in the word, and "Add Your Other Mail Account
      // Account" is what the obvious template produces. Seen in the running app.
      return state.provider.id === 'other'
        ? 'Add a Mail Account'
        : `Add Your ${state.provider.displayName} Account`
    case 'servers':
      return 'Incoming and Outgoing Servers'
    case 'testing':
      return 'Checking the Connection'
    case 'report':
      return state.report?.ok === true ? 'Account Ready' : 'Could Not Connect'
    case 'done':
      return 'Account Added'
    default:
      return 'Add an Account'
  }
}
