import { useCallback, useEffect, useReducer, useRef } from 'react'
import { AtSign, Cloud, ExternalLink, Mail, Server, Loader2 } from 'lucide-react'
import type { LucideIcon } from 'lucide-react'

import type { AccountInput } from '@/lib/generated/AccountInput'
import type { ProviderInfo } from '@/lib/generated/ProviderInfo'
import type { ServerInput } from '@/lib/generated/ServerInput'
import {
  accountAddOauth,
  accountAddPassword,
  accountDiscover,
  accountTest,
  providerOpenSetup,
} from '@/lib/ipc'
import { cx } from '@/lib/cx'
import { Button, Sheet, TextField, useToast } from '@/ui'

import { DiagnosticList } from './DiagnosticList'
import { INITIAL, canContinue, looksLikeEmail, reduce, titleFor } from './model'
import { useProviders } from './queries'
import styles from './AccountAssistant.module.css'

const PROVIDER_ICONS: Record<string, LucideIcon> = {
  google: Mail,
  microsoft: Mail,
  icloud: Cloud,
  yahoo: AtSign,
  other: Server,
}

/**
 * The account assistant. docs/04 Phase 4, modelled on Mail's.
 *
 * The order is Mail's, and it is the order for a reason: provider first, because the answer
 * changes every question after it; then the address; then servers *only* if they are not
 * already known; then the test; then the report. A single form with every field on it would
 * ask a Gmail user for an IMAP hostname the app already has.
 *
 * **Nothing is saved until the connection test passes.** An account row that cannot connect
 * appears in the sidebar and fails quietly, and working out why is then the user's problem.
 */
export interface AccountAssistantProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  /** Shown when the app has no accounts at all — the first-run path has no cancel button. */
  firstRun?: boolean
}

export function AccountAssistant({ open, onOpenChange, firstRun = false }: AccountAssistantProps) {
  const [state, dispatch] = useReducer(reduce, INITIAL)
  const providers = useProviders()
  const toast = useToast()

  // Cleared on close rather than on open, so a reopened sheet never flashes the last
  // attempt's password or diagnostic report before resetting.
  useEffect(() => {
    if (!open) dispatch({ type: 'reset' })
  }, [open])

  /* --------------------------------------------------------------- autodiscovery */

  const lookedUp = useRef('')

  useEffect(() => {
    if (state.step !== 'identity') return
    if (!looksLikeEmail(state.email)) return

    const email = state.email.trim().toLowerCase()
    if (lookedUp.current === email) return
    lookedUp.current = email

    let cancelled = false

    // Debounced, because this runs while the user is still typing the domain and the
    // lookup can reach the network. 500ms is long enough that "example.co" is not looked
    // up on the way to "example.com".
    const timer = window.setTimeout(() => {
      void accountDiscover(email).then((discovery) => {
        if (cancelled || discovery === null) return

        // A domain fronting Google or Microsoft is switched to that provider: offering a
        // password box for an account whose provider rejects passwords fails with no
        // explanation at all.
        const suggested =
          discovery.suggestedProvider !== null
            ? providers.data?.find((info) => info.id === discovery.suggestedProvider)
            : undefined

        dispatch({
          type: 'discovered',
          discovery,
          provider: suggested && suggested.id !== state.provider?.id ? suggested : undefined,
        })
      })
    }, 500)

    return () => {
      cancelled = true
      window.clearTimeout(timer)
    }
  }, [state.step, state.email, state.provider?.id, providers.data])

  /* ------------------------------------------------------------------- the test */

  const serversFor = useCallback((): { imap?: ServerInput; smtp?: ServerInput } => {
    if (state.imap === null || state.smtp === null) return {}
    return { imap: state.imap, smtp: state.smtp }
  }, [state.imap, state.smtp])

  const runTest = useCallback(async () => {
    if (state.provider === null) return

    dispatch({ type: 'busy', busy: true })

    try {
      const report = await accountTest({
        email: state.email.trim(),
        provider: state.provider.id,
        password: state.provider.authKind === 'password' ? state.password : undefined,
        ...serversFor(),
      })
      dispatch({ type: 'tested', report })
    } catch (error) {
      dispatch({ type: 'failed', message: messageOf(error) })
    }
  }, [state.provider, state.email, state.password, serversFor])

  const save = useCallback(async () => {
    if (state.provider === null) return

    const input: AccountInput = {
      displayName: state.displayName.trim(),
      email: state.email.trim(),
      provider: state.provider.id,
      imap: state.imap,
      smtp: state.smtp,
      color: null,
    }

    dispatch({ type: 'busy', busy: true })

    try {
      const added =
        state.provider.authKind === 'oAuth2'
          ? await accountAddOauth(input)
          : await accountAddPassword(input, state.password)

      if (!added.report.ok) {
        // The sign-in worked and the mailbox did not. That is a different failure from a
        // wrong password, and the report says which.
        dispatch({ type: 'tested', report: added.report })
        return
      }

      dispatch({ type: 'added' })
      toast.show({ title: `${added.email} added` })
      onOpenChange(false)
    } catch (error) {
      dispatch({ type: 'failed', message: messageOf(error) })
    }
  }, [
    state.provider,
    state.displayName,
    state.email,
    state.password,
    state.imap,
    state.smtp,
    toast,
    onOpenChange,
  ])

  // The test step has no controls of its own; entering it is what starts the work.
  useEffect(() => {
    if (state.step !== 'testing' || state.busy) return

    // An OAuth account is signed in and tested in one go, because the browser round trip
    // *is* the test — asking the user to authorise twice would be absurd.
    void (state.provider?.authKind === 'oAuth2' ? save() : runTest())
  }, [state.step, state.busy, state.provider?.authKind, runTest, save])

  /* ------------------------------------------------------------------- rendering */

  const provider = state.provider

  const footer = (
    <div className={styles.footer}>
      {state.error !== null && (
        <p className={styles.error} role="alert">
          {state.error}
        </p>
      )}

      <div className={styles.actions}>
        {!firstRun && state.step !== 'testing' && (
          <Button
            variant="bordered"
            onClick={() => {
              onOpenChange(false)
            }}
          >
            Cancel
          </Button>
        )}

        {(state.step === 'identity' || state.step === 'servers' || state.step === 'report') && (
          <Button
            variant="bordered"
            onClick={() => {
              dispatch({ type: 'back' })
            }}
          >
            Back
          </Button>
        )}

        {state.step === 'report' && state.report?.ok === true && (
          <Button
            variant="filled"
            disabled={state.busy}
            onClick={() => {
              void save()
            }}
          >
            Add Account
          </Button>
        )}

        {state.step !== 'testing' && !(state.step === 'report' && state.report?.ok === true) && (
          <Button
            variant="filled"
            disabled={!canContinue(state)}
            onClick={() => {
              dispatch({ type: 'continue' })
            }}
          >
            {state.step === 'report' ? 'Try Again' : 'Continue'}
          </Button>
        )}
      </div>
    </div>
  )

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title={titleFor(state)}
      className={styles.sheet}
      footer={footer}
    >
      <div className={styles.body}>
        {state.step === 'provider' && (
          <ProviderStep
            providers={providers.data ?? []}
            selected={provider}
            onPick={(picked) => {
              dispatch({ type: 'pickProvider', provider: picked })
            }}
          />
        )}

        {state.step === 'identity' && provider !== null && (
          <IdentityStep
            provider={provider}
            displayName={state.displayName}
            email={state.email}
            password={state.password}
            onEdit={(field, value) => {
              dispatch({ type: 'edit', field, value })
            }}
          />
        )}

        {state.step === 'servers' && (
          <ServersStep
            imap={state.imap}
            smtp={state.smtp}
            explanation={state.discovery?.explanation ?? null}
            onEdit={(which, patch) => {
              dispatch({ type: 'editServer', which, patch })
            }}
          />
        )}

        {state.step === 'testing' && (
          <div className={styles.testing}>
            <Loader2 className={styles.spinner} aria-hidden />
            <p className={styles.testingText}>
              {provider?.authKind === 'oAuth2'
                ? 'Finish signing in with your browser. Halcyon is waiting for it.'
                : 'Connecting to the incoming and outgoing servers.'}
            </p>
          </div>
        )}

        {state.step === 'report' && state.report !== null && (
          <DiagnosticList report={state.report} />
        )}
      </div>
    </Sheet>
  )
}

/* ------------------------------------------------------------------------ steps */

function ProviderStep({
  providers,
  selected,
  onPick,
}: {
  providers: ProviderInfo[]
  selected: ProviderInfo | null
  onPick: (provider: ProviderInfo) => void
}) {
  return (
    <div className={styles.step}>
      <ul className={styles.providers} role="radiogroup" aria-label="Mail account provider">
        {providers.map((provider) => {
          const Icon = PROVIDER_ICONS[provider.id] ?? Server
          const active = selected?.id === provider.id

          return (
            <li key={provider.id}>
              <button
                type="button"
                role="radio"
                aria-checked={active}
                className={cx(styles.tile, active && styles.tileActive)}
                onClick={() => {
                  onPick(provider)
                }}
              >
                <Icon className={styles.tileIcon} aria-hidden />
                <span className={styles.tileText}>
                  <span className={styles.tileName}>{provider.displayName}</span>
                  {provider.needsOauthClient && (
                    <span className={styles.tileWarning}>Needs setting up in Settings first</span>
                  )}
                </span>
              </button>
            </li>
          )
        })}
      </ul>

      {/* Always rendered, so the tiles above it never move. Standing rule 6. */}
      <div className={styles.noteSlot}>
        {selected?.setupNote !== null && selected?.setupNote !== undefined && (
          <div className={styles.note}>
            <p className={styles.noteText}>{selected.setupNote}</p>
            {selected.setupUrl !== null && (
              <Button
                variant="plain"
                icon={ExternalLink}
                onClick={() => {
                  void providerOpenSetup(selected.id)
                }}
              >
                Open in browser
              </Button>
            )}
          </div>
        )}
      </div>
    </div>
  )
}

function IdentityStep({
  provider,
  displayName,
  email,
  password,
  onEdit,
}: {
  provider: ProviderInfo
  displayName: string
  email: string
  password: string
  onEdit: (field: 'displayName' | 'email' | 'password', value: string) => void
}) {
  const touched = email.length > 0

  return (
    <div className={styles.step}>
      <TextField
        label="Name"
        autoComplete="name"
        placeholder="How your name appears on messages you send"
        value={displayName}
        onChange={(event) => {
          onEdit('displayName', event.currentTarget.value)
        }}
      />

      <TextField
        label="Email Address"
        type="email"
        autoComplete="username"
        value={email}
        invalid={touched && !looksLikeEmail(email)}
        description={
          touched && !looksLikeEmail(email)
            ? 'That does not look like an email address.'
            : undefined
        }
        onChange={(event) => {
          onEdit('email', event.currentTarget.value)
        }}
      />

      {provider.authKind === 'password' ? (
        <TextField
          label={provider.id === 'other' ? 'Password' : 'App Password'}
          type="password"
          autoComplete="current-password"
          value={password}
          description={provider.setupNote ?? undefined}
          onChange={(event) => {
            onEdit('password', event.currentTarget.value)
          }}
        />
      ) : (
        // No password field at all for an OAuth provider. There is nothing for the user to
        // type, and a box here would invite them to type their Google password into an app
        // that must never see it.
        <p className={styles.note}>{provider.setupNote}</p>
      )}
    </div>
  )
}

function ServersStep({
  imap,
  smtp,
  explanation,
  onEdit,
}: {
  imap: ServerInput | null
  smtp: ServerInput | null
  explanation: string | null
  onEdit: (which: 'imap' | 'smtp', patch: Partial<ServerInput>) => void
}) {
  return (
    <div className={styles.step}>
      {explanation !== null && <p className={styles.note}>{explanation}</p>}

      <ServerFields legend="Incoming (IMAP)" which="imap" value={imap} onEdit={onEdit} />
      <ServerFields legend="Outgoing (SMTP)" which="smtp" value={smtp} onEdit={onEdit} />
    </div>
  )
}

function ServerFields({
  legend,
  which,
  value,
  onEdit,
}: {
  legend: string
  which: 'imap' | 'smtp'
  value: ServerInput | null
  onEdit: (which: 'imap' | 'smtp', patch: Partial<ServerInput>) => void
}) {
  return (
    <fieldset className={styles.fieldset}>
      <legend className={styles.legend}>{legend}</legend>

      <div className={styles.serverRow}>
        <TextField
          label="Server"
          className={styles.host}
          value={value?.host ?? ''}
          onChange={(event) => {
            onEdit(which, { host: event.currentTarget.value })
          }}
        />

        <TextField
          label="Port"
          type="number"
          inputMode="numeric"
          className={styles.port}
          value={String(value?.port ?? '')}
          onChange={(event) => {
            const port = Number.parseInt(event.currentTarget.value, 10)
            onEdit(which, { port: Number.isNaN(port) ? 0 : port })
          }}
        />
      </div>

      <label className={styles.securityLabel}>
        <span>Encryption</span>
        <select
          className={styles.select}
          value={value?.security ?? 'tls'}
          onChange={(event) => {
            onEdit(which, { security: event.currentTarget.value })
          }}
        >
          <option value="tls">TLS/SSL</option>
          <option value="starttls">STARTTLS</option>
        </select>
      </label>
    </fieldset>
  )
}

/**
 * An error from the core is already a sentence written for a user. Anything else is a
 * thrown `Error`, whose message is not — so it gets a generic line rather than being shown
 * raw.
 */
function messageOf(error: unknown): string {
  if (typeof error === 'object' && error !== null && 'message' in error) {
    const { message } = error
    if (typeof message === 'string' && message.length > 0) return message
  }
  return 'Something went wrong. Please try again.'
}
