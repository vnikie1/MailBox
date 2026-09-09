import { useCallback, useMemo, useState } from 'react'
import { AlertTriangle, ChevronDown, ChevronUp, Plus, Trash2 } from 'lucide-react'

import type { AccountDetail } from '@/lib/generated/AccountDetail'
import {
  accountReauth,
  accountRemove,
  accountUpdate,
  accountsReorder,
  oauthClientSet,
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
export function AccountsSettings() {
  const accounts = useAccountsDetail()
  const accountsChanged = useAccountsChanged()
  const [assistantOpen, setAssistantOpen] = useState(false)
  const [removing, setRemoving] = useState<AccountDetail | null>(null)

  // Memoised because the reorder callback closes over it: a fresh array every render
  // would give that callback a new identity on every render too.
  const data = accounts.data
  const list = useMemo(() => data ?? [], [data])

  const move = useCallback(
    (index: number, direction: -1 | 1) => {
      const next = [...list]
      const target = index + direction
      const moved = next[index]
      const displaced = next[target]
      if (moved === undefined || displaced === undefined) return

      next[index] = displaced
      next[target] = moved

      void accountsReorder(next.map((account) => account.id)).then(() => {
        accountsChanged()
      })
    },
    [list, accountsChanged],
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

      <ul className={styles.list}>
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

  const commit = useCallback(() => {
    if (name.trim() === account.displayName) return
    void accountUpdate(account.id, { displayName: name.trim() }).then(() => {
      accountsChanged()
    })
  }, [account.id, account.displayName, name, accountsChanged])

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
    <li className={styles.row}>
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
                    toast.show({ title: `Signed in to ${account.email}` })
                    return syncAll()
                  })
                  .catch((cause: unknown) => {
                    toast.show({
                      title: 'That sign-in did not complete',
                      description: cause instanceof Error ? cause.message : String(cause),
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
            // Clicking the current colour clears it, which is why the core takes
            // `Option<Option<String>>` — "leave it" and "remove it" are different.
            const next = account.color === color.id ? null : color.id
            void accountUpdate(account.id, { color: next }).then(() => {
              accountsChanged()
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
              void accountRemove(account.id).then(() => {
                accountsChanged()
                toast.show({ title: `${account.email} removed` })
                onClose()
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
 * Nothing is compiled in, so this is how a Google or Microsoft account becomes possible at
 * all — and it means the app is never blocked on someone else's verification status. The
 * client id is shown back; the secret never is.
 */
function OAuthClientPanel() {
  const providers = useProviders()
  const oauthProviders = (providers.data ?? []).filter((info) => info.authKind === 'oAuth2')

  return (
    <section className={styles.advanced}>
      <h2 className={settings.heading}>Sign-in applications</h2>
      <p className={settings.intro}>
        Google and Microsoft require each app to register its own sign-in application. Halcyon ships
        without one, so you register yours and paste the client ID here. It is not a secret — it
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
        onChange={(event) => {
          setClientId(event.currentTarget.value)
        }}
      />

      {/* Labelled from the provider, not "(optional)" for everyone. Google requires the
          secret on every token refresh; Microsoft public clients have none. Calling it
          optional for Google is a lie whose cost arrives an hour later, as a refresh failure
          that reads exactly like a rejected password. */}
      <TextField
        label={`Client secret${requiresSecret ? '' : ' (optional)'}`}
        aria-label={`${label} client secret`}
        type="password"
        className={styles.clientField}
        value={clientSecret}
        invalid={requiresSecret && status.data?.configured === true && !status.data.hasSecret}
        description={
          status.data?.hasSecret === true
            ? 'A secret is saved. Type a new one to replace it.'
            : requiresSecret
              ? 'Required. Google will not refresh this account without it.'
              : undefined
        }
        onChange={(event) => {
          setClientSecret(event.currentTarget.value)
        }}
      />

      <Button
        variant="bordered"
        onClick={() => {
          void oauthClientSet(provider, value, clientSecret === '' ? undefined : clientSecret).then(
            () => {
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
            },
          )
        }}
      >
        Save
      </Button>
    </Field>
  )
}
