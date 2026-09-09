import { useEffect, useState } from 'react'

import { useAccounts } from '@/app/queries'
import { Field, Form } from '@/features/settings/SettingsForm'
import type { NotifyPrefs } from '@/lib/generated/NotifyPrefs'
import { notifyPrefs, runAtLogin, setNotifyPrefs, setRunAtLogin } from '@/lib/platform'

import styles from '@/features/settings/settings.module.css'

/**
 * Notifications and startup. docs/06 Phase 10.
 *
 * Per account, because that is the unit people actually think in: a work account and a
 * newsletter account want different answers, and one global switch means the only way to
 * silence the newsletters is to silence everything — which is what people do, and then they
 * miss the work mail the setting existed to surface.
 *
 * The address is the form label for its own group of choices rather than a heading above an
 * indented block. Same information, and it puts every account's checkboxes on the one column
 * everything else in the window is on, which is what makes two accounts comparable at a
 * glance instead of two similar-looking lists.
 */
export function NotificationSettings() {
  const { data: accounts = [] } = useAccounts()
  const [prefs, setPrefs] = useState<Record<number, NotifyPrefs>>({})
  const [startup, setStartup] = useState<boolean | null>(null)

  useEffect(() => {
    let live = true

    void runAtLogin().then((value) => {
      if (live) setStartup(value)
    })

    for (const account of accounts) {
      void notifyPrefs(account.id).then((value) => {
        if (live) setPrefs((current) => ({ ...current, [account.id]: value }))
      })
    }

    return () => {
      live = false
    }
  }, [accounts])

  const update = (accountId: number, patch: Partial<NotifyPrefs>) => {
    const current = prefs[accountId]
    if (current === undefined) return

    const next = { ...current, ...patch }
    setPrefs((all) => ({ ...all, [accountId]: next }))
    void setNotifyPrefs(accountId, next)
  }

  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Notifications</h2>

      <Form>
        {accounts.length === 0 && (
          <Field>
            <p className={styles.hint}>No accounts yet.</p>
          </Field>
        )}

        {accounts.map((account) => {
          const value = prefs[account.id]

          return (
            <Field
              key={account.id}
              // The account's name, not its address. The label column is 156px wide and an
              // address is one unbroken word — "vishal.singh@gmail.example" wrapped across two
              // lines and broke mid-domain. The address is under the choices instead, where it
              // has the whole control column and reads as the answer to "which account is
              // this?" rather than as a heading nobody can parse.
              label={account.displayName}
              hint={
                <>
                  {account.email}
                  {/* Said once, under the accounts it applies to, rather than repeated inside
                      each of them. It was in the loop, so three accounts printed the same note
                      three times — and the repetition read as three different notes that
                      happened to be identical, which is a reason to stop and compare them. */}
                  {account.id === accounts[accounts.length - 1]?.id &&
                    ' — sounds are Windows’ own, so they follow whatever you have chosen under Sound settings. The new-mail sound comes with the notification, so it needs the first option.'}
                </>
              }
            >
              <label className={styles.choice}>
                <input
                  type="checkbox"
                  className={styles.checkbox}
                  checked={value?.enabled === true}
                  disabled={value === undefined}
                  onChange={(event) => {
                    update(account.id, { enabled: event.target.checked })
                  }}
                />
                Notify me about new mail
              </label>

              <label className={styles.choice}>
                <input
                  type="checkbox"
                  className={styles.checkbox}
                  checked={value?.vipOnly === true}
                  disabled={value?.enabled !== true}
                  onChange={(event) => {
                    update(account.id, { vipOnly: event.target.checked })
                  }}
                />
                Only from VIPs
              </label>

              <label className={styles.choice}>
                <input
                  type="checkbox"
                  className={styles.checkbox}
                  checked={value?.sound === true}
                  // Not tied to the toggle above, unlike VIP-only. This covers sending as well
                  // as receiving, and a send sound has nothing to do with whether new mail is
                  // announced — disabling it here would make a setting that Rust still
                  // honours, which is the kind of disagreement nobody finds until it is
                  // confusing.
                  disabled={value === undefined}
                  onChange={(event) => {
                    update(account.id, { sound: event.target.checked })
                  }}
                />
                Play sounds when mail is sent and received
              </label>
            </Field>
          )
        })}

        <Field
          label="Startup"
          hint="Mail only arrives while Halcyon is running, so this is what makes a notification about new mail possible at all."
        >
          <label className={styles.choice}>
            <input
              type="checkbox"
              className={styles.checkbox}
              checked={startup === true}
              disabled={startup === null}
              onChange={(event) => {
                setStartup(event.target.checked)
                void setRunAtLogin(event.target.checked).then(runAtLogin).then(setStartup)
              }}
            />
            Start Halcyon when I sign in
          </label>
        </Field>
      </Form>
    </section>
  )
}
