import type { AccountDetail } from '@/lib/generated/AccountDetail'
import type { ServerSettings } from '@/lib/generated/ServerSettings'
import { Button, Sheet } from '@/ui'

import styles from './AccountInfoSheet.module.css'

/**
 * What the app knows about one account. The "Get Account Info" row of the mailbox menu.
 *
 * ## What is deliberately not here
 *
 * macOS Mail shows a quota bar and a mailbox size. Neither is drawn, because neither is known:
 * the core implements no IMAP `QUOTA` extension, and nothing sums `message.size` per mailbox.
 * An empty quota bar would be a claim the app cannot back, and standing rule 18 puts it in the
 * same category as a menu item that does nothing.
 *
 * Everything below comes from `accounts_detail`, which is the settings pane's own source and
 * carries no secret and no field that could hold one.
 */
export interface AccountInfoSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  account: AccountDetail | undefined
}

function server(settings: ServerSettings | null): string {
  if (settings === null) return 'Not configured'

  // The security is part of the address in every practical sense — the same host on two ports
  // with two different answers is the commonest mail misconfiguration there is.
  return `${settings.host}:${String(settings.port)} · ${settings.security}`
}

export function AccountInfoSheet({ open, onOpenChange, account }: AccountInfoSheetProps) {
  const rows: [string, string][] =
    account === undefined
      ? []
      : [
          ['Name', account.displayName],
          ['Address', account.email],
          ['Provider', account.provider],
          ['Sign-in', account.authKind === 'oAuth2' ? 'OAuth' : 'Password'],
          ['Incoming (IMAP)', server(account.imap)],
          ['Outgoing (SMTP)', server(account.smtp)],
          ['Syncing', account.syncEnabled ? 'On' : 'Off'],
          [
            'Credential',
            account.hasCredential
              ? 'Stored'
              : 'Missing — this account cannot connect until you sign in again',
          ],
        ]

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title="Account Information"
      footer={
        <Button
          onClick={() => {
            onOpenChange(false)
          }}
        >
          Done
        </Button>
      }
    >
      <dl className={styles.list}>
        {rows.map(([label, value]) => (
          <div key={label} className={styles.row}>
            <dt className={styles.label}>{label}</dt>
            <dd className={styles.value}>{value}</dd>
          </div>
        ))}
      </dl>
    </Sheet>
  )
}
