import { useEffect, useState } from 'react'

import { useAccounts } from '@/app/queries'
import type { NotifyPrefs } from '@/lib/generated/NotifyPrefs'
import { notifyPrefs, runAtLogin, setNotifyPrefs, setRunAtLogin } from '@/lib/platform'

import styles from './NotificationSettings.module.css'

/**
 * Notifications and startup. docs/06 Phase 10.
 *
 * Per account, because that is the unit people actually think in: a work account and a
 * newsletter account want different answers, and one global switch means the only way to
 * silence the newsletters is to silence everything — which is what people do, and then they
 * miss the work mail the setting existed to surface.
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
      <h3 className={styles.heading}>Notifications</h3>

      {accounts.length === 0 && <p className={styles.hint}>No accounts yet.</p>}

      {accounts.map((account) => {
        const value = prefs[account.id]

        return (
          <div key={account.id} className={styles.account}>
            <span className={styles.name}>{account.email}</span>

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
                disabled={!value?.enabled}
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
                disabled={!value?.enabled}
                onChange={(event) => {
                  update(account.id, { sound: event.target.checked })
                }}
              />
              Play a sound
            </label>
          </div>
        )
      })}

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

      <p className={styles.hint}>
        Mail only arrives while Halcyon is running. Starting it at sign-in is what makes a
        notification about new mail possible at all.
      </p>
    </section>
  )
}
