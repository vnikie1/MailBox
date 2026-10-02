import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { AlertTriangle, ChevronDown, ChevronUp, Plus, Trash2 } from 'lucide-react'

import type { AccountDetail } from '@/lib/generated/AccountDetail'
import type { OAuthClientStatus } from '@/lib/generated/OAuthClientStatus'
import {
  accountReauth,
  accountRemove,
  accountUpdate,
  accountsReorder,
  oauthClientSet,
  reasonFor,
  syncAll,
} from '@/lib/ipc'
import { Field, Form } from '@/features/settings/SettingsForm'
import { cx } from '@/lib/cx'
import { Avatar, Button, IconButton, Sheet, TextField, useToast } from '@/ui'

import { AccountAssistant } from './AccountAssistant'
import { useAccountsDetail, useOAuthClient, useProviders } from './queries'
import { useAccountsChanged } from './useAccountsChanged'
import styles from './AccountsSettings.module.css'
import settings from '@/features/settings/settings.module.css'

/** The flag palette, which is the only colour set docs/02 §2 allows outside the accent. */
const COLORS: { id: string; label: string }[] = [
  { id: 'red', label: 'Red' },
  { id: 'orange', label: 'Orange' },
  { id: 'yellow', label: 'Yellow' },
  { id: 'green', label: 'Green' },
  { id: 'blue', label: 'Blue' },
  { id: 'purple', label: 'Purple' },
  { id: 'gray', label: 'Grey' },
]

/**
 * Settings → Accounts. docs/04 Phase 4 — *multi-account, reordering, per-account colour,
 * remove with purge*.
 *
 * Removal is the part worth being careful about, and it gets a confirmation that says what
 * will actually happen: the mail goes, and so does the saved password. "Remove account" on
 * its own does not tell a user that their downloaded mail is about to be deleted.
 */
export interface AccountsSettingsProps {
  /**
   * An account to bring into view with its name field focused — the mailbox menu's
   * `Edit "Account"…`. `request` changes on every ask, so asking again for the same account
   * works too.
   */
  focus?: { accountId: number; request: number } | null
}

export function AccountsSettings({ focus = null }: AccountsSettingsProps) {
  const accounts = useAccountsDetail()
  const accountsChanged = useAccountsChanged()
  const toast = useToast()
  const [assistantOpen, setAssistantOpen] = useState(false)
  const [removing, setRemoving] = useState<AccountDetail | null>(null)
  const rows = useRef<HTMLUListElement>(null)

  // Memoised because the reorder callback closes over it: a fresh array every render
  // would give that callback a new identity on every render too.
  const data = accounts.data
  const list = useMemo(() => data ?? [], [data])

  // Waits for the list: the pane opens before its query answers, and the row asked for does
  // not exist until then.
  const loaded = list.length > 0
  useEffect(() => {
    if (focus === null || !loaded) return

    const row = rows.current?.querySelector<HTMLElement>(
      `[data-account-id="${String(focus.accountId)}"]`,
    )
    if (!row) return

    row.scrollIntoView({ block: 'nearest' })
    row.querySelector<HTMLInputElement>('input')?.focus()
  }, [focus, loaded])

  const move = useCallback(
    (index: number, direction: -1 | 1) => {
      const next = [...list]
      const target = index + direction
      const moved = next[index]
      const displaced = next[target]
      if (moved === undefined || displaced === undefined) return

      next[index] = displaced
      next[target] = moved

      accountsReorder(next.map((account) => account.id))
        .then(() => {
          accountsChanged()
        })
        .catch((cause: unknown) => {
          toast.show({
            title: 'That order was not saved',
            description: reasonFor(cause),
          })
        })
    },
    [list, accountsChanged, toast],
  )

  return (
    <div className={styles.wrap}>
      <header className={styles.header}>
        <h2 className={settings.heading}>Mail accounts</h2>
        <Button
          variant="bordered"
          icon={Plus}
          onClick={() => {
            setAssistantOpen(true)
          }}
        >
          Add Account
        </Button>
      </header>

      {list.length === 0 && (
        <p className={styles.empty}>No accounts yet. Add one to start receiving mail.</p>
      )}

      <ul ref={rows} className={styles.list}>
        {list.map((account, index) => (
          <AccountRow
            key={account.id}
            account={account}
            first={index === 0}
            last={index === list.length - 1}
            onMove={(direction) => {
              move(index, direction)
            }}
            onRemove={() => {
              setRemoving(account)
            }}
          />
        ))}
      </ul>

      <OAuthClientPanel />

      <AccountAssistant open={assistantOpen} onOpenChange={setAssistantOpen} />

      <RemoveConfirmation
        account={removing}
        onClose={() => {
          setRemoving(null)
        }}
      />
    </div>
  )
}

function AccountRow({
  account,
  first,
  last,
  onMove,
  onRemove,
}: {
  account: AccountDetail
  first: boolean
  last: boolean
  onMove: (direction: -1 | 1) => void
  onRemove: () => void
}) {
  const [name, setName] = useState(account.displayName)
  // The browser sign-in takes as long as the user takes, so the button has to say it is busy
  // or it reads as not having worked.
  const [signingIn, setSigningIn] = useState(false)
  const toast = useToast()
  const accountsChanged = useAccountsChanged()

  /**
   * The rename, and the one failure here that lies rather than does nothing.
   *
   * `name` is local state seeded once from the account, and the row's key does not change when
   * a rename is rejected — so nothing puts the field back. Without the catch, a refused rename
   * leaves the typed text sitting in the box looking saved while the database still holds the
   * old one. Every other control on this row renders from server data and merely fails to
   * move; this one shows an answer that is not true, which is worse.
   */
  const commit = useCallback(() => {
    if (name.trim() === account.displayName) return
    accountUpdate(account.id, { displayName: name.trim() })
      .then(() => {
        accountsChanged()
      })
      .catch((cause: unknown) => {
        setName(account.displayName)
        toast.show({
          title: 'That name was not saved',
          description: reasonFor(cause),
        })
      })
  }, [account.id, account.displayName, name, accountsChanged, toast])

  /**
   * Three lines, not one.
   *
   * The avatar, an editable name, a sign-in button, seven colour dots, two reorder arrows and
   * a delete used to share a single row about 700px wide. The address lost — it was the one
   * thing in the row with no natural length, so it truncated to "vnikie1…" while the name
   * field beside it sat at a width narrower than the word in it. Nothing there was optional,
   * so the row had to become taller rather than the contents smaller.
   *
   * Name and the destructive actions on the first line; the address, whole, on the second; the
   * colour and the sign-in on the third, which is where the things you touch rarely belong.
   */
  return (
    <li className={styles.row} data-account-id={account.id}>
      <Avatar name={account.displayName} email={account.email} size="md" />

      <div className={styles.identity}>
        <div className={styles.topLine}>
          <TextField
            label="Description"
            hideLabel
            className={styles.nameField}
            value={name}
            onChange={(event) => {
              setName(event.currentTarget.value)
            }}
            onBlur={commit}
          />

          <div className={styles.rowActions}>
            <IconButton
              icon={ChevronUp}
              label={`Move ${account.displayName} up`}
              disabled={first}
              onClick={() => {
                onMove(-1)
              }}
            />
            <IconButton
              icon={ChevronDown}
              label={`Move ${account.displayName} down`}
              disabled={last}
              onClick={() => {
                onMove(1)
              }}
            />
            <IconButton icon={Trash2} label={`Remove ${account.displayName}`} onClick={onRemove} />
          </div>
        </div>

        <span className={styles.email}>{account.email}</span>

        <div className={styles.bottomLine}>
          <ColorPicker account={account} />

          {/* docs/03 §7 — an account with no stored credential cannot connect, and saying so
              here is the difference between "broken" and "sign in again".

              It said it and offered no way to do it: this was a `<span>`, and there was no
              command behind it either. A stored credential that the *server* refuses does not
              clear `hasCredential` at all, so for the common case — an expired refresh token —
              even the words were absent. The button is offered for every OAuth account, and
              only wears the warning colour when the credential is actually missing. */}
          {account.authKind === 'oAuth2' && (
            <Button
              variant="plain"
              className={account.hasCredential ? styles.signIn : styles.reauth}
              disabled={signingIn}
              onClick={() => {
                setSigningIn(true)
                accountReauth(account.id)
                  .then(() => {
                    // No sync from here. The core announces `accounts:changed`, and the main
                    // window answers it by fetching mail and resuming the watcher; this used to
                    // add a `syncAll()` of its own, which synced every account a second time.
                    toast.show({ title: `Signed in to ${account.email}` })
                  })
                  .catch((cause: unknown) => {
                    toast.show({
                      title: 'That sign-in did not complete',
                      // Not `instanceof Error`: the core rejects with a plain `{code, message}`
                      // object, so that idiom printed "[object Object]" for precisely the
                      // errors this toast exists to show. See `reasonFor`.
                      description: reasonFor(cause),
                    })
                  })
                  .finally(() => {
                    setSigningIn(false)
                  })
              }}
            >
              {!account.hasCredential && (
                <AlertTriangle className={styles.reauthIcon} aria-hidden />
              )}
              {signingIn ? 'Signing in…' : 'Sign in again'}
            </Button>
          )}
        </div>
      </div>
    </li>
  )
}

function ColorPicker({ account }: { account: AccountDetail }) {
  const accountsChanged = useAccountsChanged()
  const toast = useToast()

  return (
    <div
      className={styles.colors}
      role="radiogroup"
      aria-label={`Colour for ${account.displayName}`}
    >
      {COLORS.map((color) => (
        <button
          key={color.id}
          type="button"
          role="radio"
          aria-checked={account.color === color.id}
          aria-label={color.label}
          className={cx(styles.swatch, account.color === color.id && styles.swatchActive)}
          data-color={color.id}
          onClick={() => {
            // Clicking the current colour clears it, which is why the core takes a
            // `ColorChange` rather than a bare string — "leave it" and "remove it" are
            // different, and a nested option could not express the difference over JSON.
            const next = account.color === color.id ? null : color.id
            accountUpdate(account.id, { color: next })
              .then(() => {
                accountsChanged()
              })
              .catch((cause: unknown) => {
                toast.show({
                  title: 'That colour was not saved',
                  description: reasonFor(cause),
                })
              })
          }}
        />
      ))}
    </div>
  )
}

function RemoveConfirmation({
  account,
  onClose,
}: {
  account: AccountDetail | null
  onClose: () => void
}) {
  const toast = useToast()
  const accountsChanged = useAccountsChanged()

  return (
    <Sheet
      open={account !== null}
      onOpenChange={(open) => {
        if (!open) onClose()
      }}
      title={account === null ? 'Remove Account' : `Remove ${account.displayName}?`}
      footer={
        <div className={styles.confirmActions}>
          <Button variant="bordered" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="destructive"
            onClick={() => {
              if (account === null) return
              // The sheet closes only on success, so a refusal has to say so — otherwise the
              // one destructive button in this window looks simply inert, and the user cannot
              // tell whether the account went or not.
              accountRemove(account.id)
                .then(() => {
                  accountsChanged()
                  toast.show({ title: `${account.email} removed` })
                  onClose()
                })
                .catch((cause: unknown) => {
                  toast.show({
                    title: `${account.email} was not removed`,
                    description: reasonFor(cause),
                  })
                })
            }}
          >
            Remove Account
          </Button>
        </div>
      }
    >
      {/* Said plainly, because "remove account" does not tell anyone their downloaded mail
          is about to be deleted, and it is not recoverable from here. */}
      <p className={styles.confirmBody}>
        Every message downloaded for {account?.email} will be deleted from this computer, and the
        saved password will be removed from Windows Credential Manager.
      </p>
      <p className={styles.confirmBody}>
        Nothing is deleted from the mail server. Adding the account again will download it afresh.
      </p>
    </Sheet>
  )
}

/**
 * docs/05 §2's "bring your own OAuth client".
 *
 * A build may carry its own client for each provider (`src-tauri/oauth/`), and then this panel
 * is optional: it is for someone who wants an application of their own instead. A build from
 * public source carries none, and then this is the only way a Google or Microsoft account
 * becomes possible at all. The intro has to be true of both, so it describes the rule rather
 * than asserting which kind of build this is — each provider's row says which applies to it.
 *
 * The client id is shown back; the secret never is.
 */
function OAuthClientPanel() {
  const providers = useProviders()
  const oauthProviders = (providers.data ?? []).filter((info) => info.authKind === 'oAuth2')

  return (
    <section className={styles.advanced}>
      <h2 className={settings.heading}>Sign-in applications</h2>
      <p className={settings.intro}>
        Google and Microsoft only let a registered application sign you in. Where Halcyon has one
        built in, it is used automatically and you can leave these fields empty. To use an
        application you registered yourself, paste its client ID here — it is not a secret; it
        appears in the address bar when you sign in.
      </p>

      <Form>
        {oauthProviders.map((provider) => (
          <OAuthClientFields
            key={provider.id}
            provider={provider.id}
            label={provider.displayName}
            requiresSecret={provider.requiresClientSecret}
          />
        ))}
      </Form>
    </section>
  )
}

/**
 * What the Client ID field says about the client **in use**, not about the box.
 *
 * In a build that carries its own application an empty box is the normal, working state. Every
 * word here used to assume the opposite — that an empty box meant a provider nobody could sign in
 * to — which in such a build would read as a fault on a setup that works.
 */
function clientIdDescription(
  status: OAuthClientStatus | undefined,
  label: string,
): string | undefined {
  if (status === undefined) return undefined

  switch (status.source) {
    case 'builtin':
      return "Halcyon's own application is in use. Leave this empty to keep using it."
    case 'custom':
      // Without a built-in there is nothing to go back to, and nothing worth saying.
      return status.builtin
        ? "Your own application is in use. Clear this and save to go back to Halcyon's."
        : undefined
    case null:
      return `Needed before a ${label} account can be added.`
  }
}

function clientSecretDescription(status: OAuthClientStatus | undefined, label: string): string {
  if (status?.hasSecret === true) return 'A secret is saved. Type a new one to replace it.'
  if (status?.source === 'builtin') return 'Only needed with an application of your own.'
  return `Required. ${label} will not refresh an account without it.`
}

function OAuthClientFields({
  provider,
  label,
  requiresSecret,
}: {
  provider: string
  label: string
  requiresSecret: boolean
}) {
  const status = useOAuthClient(provider)
  const [clientId, setClientId] = useState<string | null>(null)
  const [clientSecret, setClientSecret] = useState('')
  const toast = useToast()
  const accountsChanged = useAccountsChanged()

  const value = clientId ?? status.data?.clientId ?? ''

  /**
   * The provider's name is the form label; the two fields and the Save sit in the control
   * column under it.
   *
   * They used to be three columns of their own — "Google client ID", "Google client secret"
   * and a Save button — inside a settings window whose every other control lines up on one
   * axis. Only Google's secret carries a description, so that field was a line and a half
   * taller than its neighbour and the Save button, offset by hand to clear a label, landed
   * level with nothing in the Microsoft row underneath. Stacking them puts every one of these
   * controls on the same left edge as the rest of the window.
   */
  return (
    <Field label={label} htmlFor={`${provider}-client-id`}>
      {/* Short label, long name. The visible text does not need to repeat the provider — it is
          the row's own label, a column to the left — but two fields both announced as "Client
          ID" would be two controls a screen reader cannot tell apart. */}
      <TextField
        id={`${provider}-client-id`}
        label="Client ID"
        aria-label={`${label} client ID`}
        className={styles.clientField}
        value={value}
        description={clientIdDescription(status.data, label)}
        onChange={(event) => {
          setClientId(event.currentTarget.value)
        }}
      />

      {/* Only for a provider that uses one — which is Google alone.

          It used to be offered to every provider, labelled "(optional)" for Microsoft. Halcyon
          registers a Microsoft app as a *public* client, and a public client that presents a
          secret is refused (AADSTS700025), so the box invited exactly the input that would
          break sign-in. The core no longer sends one either; see `oauth::exchange`.

          For Google it is not optional, and saying so matters: a missing secret surfaces an
          hour later as a refresh failure that reads exactly like a rejected password. */}
      {requiresSecret && (
        <TextField
          label="Client secret"
          aria-label={`${label} client secret`}
          type="password"
          className={styles.clientField}
          value={clientSecret}
          // From the core, not worked out here: whether the client *in use* can refresh a
          // token is not the same as whether the user typed a secret. A client whose id is the
          // built-in one borrows the built-in secret, and that id is not visible from here.
          invalid={status.data?.missingSecret === true}
          description={clientSecretDescription(status.data, label)}
          onChange={(event) => {
            setClientSecret(event.currentTarget.value)
          }}
        />
      )}

      <Button
        variant="bordered"
        onClick={() => {
          oauthClientSet(provider, value, clientSecret === '' ? undefined : clientSecret)
            .then(() => {
              // Cleared from the form the moment it is stored. There is no reason for a
              // secret to sit in a React state tree after it has been handed to Windows.
              setClientSecret('')
              accountsChanged()

              // Retry immediately. Someone who has just pasted a credential has done so
              // *because* an account was failing, and leaving them to work out that nothing
              // will happen until the next launch is the sort of gap that reads as the fix
              // not having worked. Accounts that are already fine cost one no-op.
              void syncAll()

              toast.show({ title: `${label} sign-in application saved` })
            })
            .catch((cause: unknown) => {
              // The secret is deliberately NOT cleared here: it was not stored, and wiping the
              // box would make the user paste it again to find out why.
              toast.show({
                title: `${label} sign-in application was not saved`,
                description: reasonFor(cause),
              })
            })
        }}
      >
        Save
      </Button>
    </Field>
  )
}
