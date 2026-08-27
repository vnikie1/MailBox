/**
 * The IPC client. docs/03-architecture.md §4.
 *
 * The UI reaches the core through this module and nowhere else — standing rule 9 means
 * there is never a network call on this side of the seam, and standing rule 14 means
 * changes arrive as events rather than by polling.
 *
 * Every entry point also has a browser path. That is not a mock of the app: it is what
 * the shell genuinely is when served by Vite rather than hosted in a WebView, and it is
 * what Playwright and the Phase 1 component gallery run against. Where the browser can
 * answer honestly (the OS theme, reduced transparency, window focus) it does; where it
 * cannot (a DWM material) it reports the truth, which is that there is none.
 */

import { invoke, isTauri } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { getCurrentWindow } from '@tauri-apps/api/window'

import { DEFAULT_APPEARANCE, type Appearance, type ThemeName } from './appearance'

export const runningInTauri: boolean = isTauri()

/* ------------------------------------------------------------------ appearance */

function browserAppearance(): Appearance {
  const dark = window.matchMedia('(prefers-color-scheme: dark)').matches
  const reduceTransparency = window.matchMedia('(prefers-reduced-transparency: reduce)').matches
  return {
    ...DEFAULT_APPEARANCE,
    theme: (dark ? 'dark' : 'light') satisfies ThemeName,
    reduceTransparency,
  }
}

export async function getAppearance(): Promise<Appearance> {
  if (!runningInTauri) return browserAppearance()
  return invoke<Appearance>('appearance_get')
}

export async function onAppearanceChanged(
  handler: (appearance: Appearance) => void,
): Promise<UnlistenFn> {
  if (!runningInTauri) {
    const queries = [
      window.matchMedia('(prefers-color-scheme: dark)'),
      window.matchMedia('(prefers-reduced-transparency: reduce)'),
    ]
    const relay = () => {
      handler(browserAppearance())
    }
    queries.forEach((q) => {
      q.addEventListener('change', relay)
    })
    return () => {
      queries.forEach((q) => {
        q.removeEventListener('change', relay)
      })
    }
  }
  return listen<Appearance>('system:appearance', (event) => {
    handler(event.payload)
  })
}

/* --------------------------------------------------------------- window chrome */

export async function onWindowFocusChanged(
  handler: (focused: boolean) => void,
): Promise<UnlistenFn> {
  if (!runningInTauri) {
    const onFocus = () => {
      handler(true)
    }
    const onBlur = () => {
      handler(false)
    }
    window.addEventListener('focus', onFocus)
    window.addEventListener('blur', onBlur)
    return () => {
      window.removeEventListener('focus', onFocus)
      window.removeEventListener('blur', onBlur)
    }
  }
  return getCurrentWindow().onFocusChanged(({ payload }) => {
    handler(payload)
  })
}

/* -------------------------------------------------------------------------- mail */

import type { AccountRow } from './generated/AccountRow'
import type { FlagPatch } from './generated/FlagPatch'
import type { ListQuery } from './generated/ListQuery'
import type { MailboxRow } from './generated/MailboxRow'
import type { MessageFull } from './generated/MessageFull'
import type { MessageRow } from './generated/MessageRow'
import type { Page } from './generated/Page'
import type { SearchQuery } from './generated/SearchQuery'

import * as browser from '@/mock/browserStore'

/**
 * The mail commands. docs/03-architecture.md §4.
 *
 * Every signature here comes from `./generated/`, which `cargo test` writes from the Rust
 * types. Renaming a field on one side without the other is a TypeScript error rather than
 * an `undefined` discovered at runtime.
 *
 * Each has a browser path, backed by `src/mock/browserStore.ts`. That is not a mock of the
 * app — it is what the app genuinely is when served by Vite instead of hosted in a WebView,
 * and it is what the Playwright suite drives. The two implementations match deliberately,
 * down to the keyset cursor semantics.
 */

export async function accountsList(): Promise<AccountRow[]> {
  if (!runningInTauri) return browser.accountsList()
  return invoke<AccountRow[]>('accounts_list')
}

export async function mailboxesTree(accountId?: number): Promise<MailboxRow[]> {
  if (!runningInTauri) return browser.mailboxesTree(accountId)
  return invoke<MailboxRow[]>('mailboxes_tree', { accountId: accountId ?? null })
}

export async function messagesPage(query: ListQuery): Promise<Page<MessageRow>> {
  if (!runningInTauri) return browser.messagesPage(query)
  return invoke<Page<MessageRow>>('messages_page', { query })
}

export async function messageGet(id: number): Promise<MessageFull | null> {
  if (!runningInTauri) return browser.messageGet(id)
  return invoke<MessageFull | null>('message_get', { id })
}

export async function threadGet(threadId: number): Promise<MessageFull[]> {
  if (!runningInTauri) return browser.threadGet(threadId)
  return invoke<MessageFull[]>('thread_get', { threadId })
}

export async function searchMessages(query: SearchQuery): Promise<MessageRow[]> {
  if (!runningInTauri) return browser.search(query)
  return invoke<MessageRow[]>('search', { query })
}

export async function msgSetFlags(ids: number[], patch: FlagPatch): Promise<number> {
  if (!runningInTauri) return browser.setFlags(ids, patch).changed
  return invoke<number>('msg_set_flags', { ids, patch })
}

export async function msgMove(ids: number[], mailboxId: number): Promise<number> {
  if (!runningInTauri) return browser.moveTo(ids, mailboxId).changed
  return invoke<number>('msg_move', { ids, mailboxId })
}

export async function msgDelete(ids: number[], permanent: boolean): Promise<number> {
  if (!runningInTauri) return browser.remove(ids, permanent).changed
  return invoke<number>('msg_delete', { ids, permanent })
}

export interface MailboxChanged {
  mailboxId: number
  unread: number
  total: number
}

/**
 * The core pushes these; the UI invalidates query keys in response and never polls
 * (standing rule 14).
 *
 * In the browser there is no core to push, so mutations there notify through the same
 * channel synchronously — the UI cannot tell the difference, which is the point.
 */
type Listener<T> = (payload: T) => void

const browserBus = new EventTarget()

export function notifyBrowserMailboxChange(mailboxIds: number[]): void {
  if (runningInTauri) return
  for (const entry of browser.mailboxCounts(mailboxIds)) {
    browserBus.dispatchEvent(new CustomEvent('mailbox:changed', { detail: entry }))
  }
}

export async function onMailboxChanged(handler: Listener<MailboxChanged>): Promise<UnlistenFn> {
  if (!runningInTauri) {
    const relay = (event: Event) => {
      handler((event as CustomEvent<MailboxChanged>).detail)
    }
    browserBus.addEventListener('mailbox:changed', relay)
    return () => {
      browserBus.removeEventListener('mailbox:changed', relay)
    }
  }

  return listen<MailboxChanged>('mailbox:changed', (event) => {
    handler(event.payload)
  })
}

export async function onMessagesUpdated(handler: Listener<number[]>): Promise<UnlistenFn> {
  if (!runningInTauri) return () => undefined
  return listen<number[]>('messages:updated', (event) => {
    handler(event.payload)
  })
}

/**
 * What "now" means to the store the UI is talking to.
 *
 * In the app this is the clock, because the mail has real dates. In the browser it is the
 * instant the browser fixtures were generated relative to — a real clock there would put
 * every fixture in the past and empty the Today section, and the visual baselines would
 * shift every time they were regenerated.
 *
 * One function rather than a conditional at each call site, so the two halves cannot
 * disagree about which day it is.
 */
export function storeNow(): Date {
  return runningInTauri ? new Date() : browser.BROWSER_NOW
}

/* --------------------------------------------------------------------- accounts */

import type { AccountDetail } from './generated/AccountDetail'
import type { AccountInput } from './generated/AccountInput'
import type { AddedAccount } from './generated/AddedAccount'
import type { DiagnosticReport } from './generated/DiagnosticReport'
import type { DiscoveryResult } from './generated/DiscoveryResult'
import type { OAuthClientStatus } from './generated/OAuthClientStatus'
import type { ProviderInfo } from './generated/ProviderInfo'
import type { ServerInput } from './generated/ServerInput'

/**
 * The account commands. docs/04 Phase 4.
 *
 * **No function here carries a secret out of the core.** A password goes in, to
 * `accountAddPassword` and `accountTest`, and is never returned by anything — there is no
 * `credentialGet` to call. Standing rule 12 holds on this side of the seam as well.
 *
 * The browser path is deliberately partial, and says so rather than pretending. A page
 * served by Vite has no Credential Manager and no way to open a TLS connection to port 993,
 * so the two commands that need those refuse in plain words. Everything that is genuinely
 * possible in a browser — the provider list, the known-domain lookups, editing and removing
 * accounts in the in-memory store — works for real, which is what the Playwright suite
 * drives.
 */

export async function providersList(): Promise<ProviderInfo[]> {
  if (!runningInTauri) return browser.providersList()
  return invoke<ProviderInfo[]>('providers_list')
}

export async function accountDiscover(email: string): Promise<DiscoveryResult | null> {
  if (!runningInTauri) return browser.accountDiscover(email)
  return invoke<DiscoveryResult | null>('account_discover', { email })
}

export interface TestRequest {
  email: string
  provider: string
  password?: string | undefined
  imap?: ServerInput | undefined
  smtp?: ServerInput | undefined
}

export async function accountTest(request: TestRequest): Promise<DiagnosticReport> {
  if (!runningInTauri) return browser.accountTest()
  return invoke<DiagnosticReport>('account_test', {
    email: request.email,
    provider: request.provider,
    password: request.password ?? null,
    imap: request.imap ?? null,
    smtp: request.smtp ?? null,
  })
}

export async function accountAddPassword(
  input: AccountInput,
  password: string,
): Promise<AddedAccount> {
  if (!runningInTauri) return browser.accountAdd()
  return invoke<AddedAccount>('account_add_password', { input, password })
}

export async function accountAddOauth(input: AccountInput): Promise<AddedAccount> {
  if (!runningInTauri) return browser.accountAdd()
  return invoke<AddedAccount>('account_add_oauth', { input })
}

export async function accountsDetail(): Promise<AccountDetail[]> {
  if (!runningInTauri) return browser.accountsDetail()
  return invoke<AccountDetail[]>('accounts_detail')
}

export async function accountUpdate(
  id: number,
  patch: {
    displayName?: string | undefined
    color?: string | null | undefined
    syncEnabled?: boolean | undefined
  },
): Promise<void> {
  if (!runningInTauri) {
    browser.accountUpdate(id, patch)
    return
  }
  await invoke('account_update', {
    id,
    displayName: patch.displayName ?? null,
    // `undefined` means "leave it alone" and `null` means "clear it", which the core reads
    // as `Option<Option<String>>`. Collapsing the two here would make the colour
    // unclearable.
    color: patch.color === undefined ? null : [patch.color],
    syncEnabled: patch.syncEnabled ?? null,
  })
}

export async function accountsReorder(ids: number[]): Promise<void> {
  if (!runningInTauri) {
    browser.accountsReorder(ids)
    return
  }
  await invoke('accounts_reorder', { ids })
}

export async function accountRemove(id: number): Promise<void> {
  if (!runningInTauri) {
    browser.accountRemove(id)
    return
  }
  await invoke('account_remove', { id })
}

export async function oauthClientGet(provider: string): Promise<OAuthClientStatus> {
  if (!runningInTauri) return browser.oauthClientGet(provider)
  return invoke<OAuthClientStatus>('oauth_client_get', { provider })
}

export async function oauthClientSet(
  provider: string,
  clientId: string,
  clientSecret?: string,
): Promise<void> {
  if (!runningInTauri) {
    browser.oauthClientSet(provider, clientId)
    return
  }
  await invoke('oauth_client_set', {
    provider,
    clientId,
    clientSecret: clientSecret ?? null,
  })
}

export async function providerOpenSetup(provider: string): Promise<void> {
  if (!runningInTauri) {
    // A browser can open its own tab, and doing the real thing is better than refusing.
    const url = browser.providerSetupUrl(provider)
    if (url !== null) window.open(url, '_blank', 'noopener,noreferrer')
    return
  }
  await invoke('provider_open_setup', { provider })
}

export async function onAccountsChanged(handler: () => void): Promise<UnlistenFn> {
  if (!runningInTauri) {
    const relay = () => {
      handler()
    }
    browserBus.addEventListener('accounts:changed', relay)
    return () => {
      browserBus.removeEventListener('accounts:changed', relay)
    }
  }
  return listen('accounts:changed', () => {
    handler()
  })
}

export function notifyBrowserAccountsChanged(): void {
  if (runningInTauri) return
  browserBus.dispatchEvent(new Event('accounts:changed'))
}

/* ------------------------------------------------------------------------- sync */

export interface SyncProgress {
  accountId: number
  mailbox: string
  written: number
  usable: boolean
  done: boolean
}

export interface SyncAccountError {
  accountId: number
  message: string
  retryInSeconds: number
  needsReauth: boolean
}

export interface SyncActivity {
  accountId: number
  /** True for a real IDLE notification, false for a tick of the polling fallback. */
  live: boolean
}

/**
 * The sync commands. docs/06 Phase 5.
 *
 * Both return as soon as the work is *scheduled*. A first sync of a large mailbox takes
 * minutes, and an IPC call that blocked for minutes would make the window look hung — the
 * events below are how the UI follows along (standing rule 14: events, never polling).
 *
 * There is no browser path. A page served by Vite cannot open an IMAP connection, and a
 * fake one would be a command that returns a plausible shape and does nothing — standing
 * rule 18. The browser store's mail is generated, not synced, and says so.
 */
export async function syncNow(accountId: number): Promise<void> {
  if (!runningInTauri) return
  await invoke('sync_now', { accountId })
}

export async function syncAll(): Promise<void> {
  if (!runningInTauri) return
  await invoke('sync_all')
}

/**
 * Starts the per-account IDLE watchers, so new mail arrives without being asked for.
 *
 * Idempotent: call it on launch and again whenever accounts change rather than tracking what
 * is already running. Starting a second watcher for an account would double every
 * notification and hold a connection the account's budget does not have — the core checks,
 * but the caller should not rely on that being the only guard.
 */
export async function syncWatch(): Promise<void> {
  if (!runningInTauri) return
  await invoke('sync_watch')
}

/**
 * Fires when the server itself reports a change — a real IDLE notification, or a tick of the
 * polling fallback on a server without IDLE.
 */
export async function onSyncActivity(
  handler: (activity: SyncActivity) => void,
): Promise<UnlistenFn> {
  if (!runningInTauri) return () => undefined
  return listen<SyncActivity>('sync:activity', (event) => {
    handler(event.payload)
  })
}

export async function onSyncProgress(
  handler: (progress: SyncProgress) => void,
): Promise<UnlistenFn> {
  if (!runningInTauri) return () => undefined
  return listen<SyncProgress>('sync:progress', (event) => {
    handler(event.payload)
  })
}

export async function onMessagesAdded(handler: (mailboxId: number) => void): Promise<UnlistenFn> {
  if (!runningInTauri) return () => undefined
  return listen<number>('messages:added', (event) => {
    handler(event.payload)
  })
}

export async function onMailboxesChanged(
  handler: (accountId: number) => void,
): Promise<UnlistenFn> {
  if (!runningInTauri) return () => undefined
  return listen<number>('mailboxes:changed', (event) => {
    handler(event.payload)
  })
}

export async function onAccountError(
  handler: (error: SyncAccountError) => void,
): Promise<UnlistenFn> {
  if (!runningInTauri) return () => undefined
  return listen<SyncAccountError>('account:error', (event) => {
    handler(event.payload)
  })
}

/**
 * Downloads the bodies for these messages if they are not already cached.
 *
 * docs/06 Phase 5 §3 — lazy fetch on selection, plus a prefetch of the next three rows.
 * Messages already held cost nothing, so the caller need not track what it has.
 *
 * Returns once the work is scheduled; `messages:updated` says it arrived.
 */
export async function bodiesEnsure(accountId: number, messageIds: number[]): Promise<void> {
  if (!runningInTauri) return
  if (messageIds.length === 0) return
  await invoke('bodies_ensure', { accountId, messageIds })
}

/* ---------------------------------------------------------------- message rendering */

import type { HostMismatch } from './generated/HostMismatch'
import type { LinkOutcome } from './generated/LinkOutcome'
import type { Rendered } from './generated/Rendered'
import type { AttachmentData } from './generated/AttachmentData'
import type { OutgoingMessage } from './generated/OutgoingMessage'
import type { OutboxRow } from './generated/OutboxRow'
import type { ReplyDraft } from './generated/ReplyDraft'

/**
 * A message body, sanitised and ready for the sandboxed frame. docs/03 §6.
 *
 * This is the **only** way message HTML reaches the UI, and the core sanitises on every call.
 * There is deliberately no command that returns the stored HTML unprocessed — the sanitiser
 * cannot be forgotten because there is nothing else to call.
 *
 * `loadRemote` is the user's explicit, per-message consent.
 */
export async function messageBody(messageId: number, loadRemote: boolean): Promise<Rendered> {
  if (!runningInTauri) {
    // A browser has no `.eml` cache and no core to proxy through. Refusing is honest;
    // rendering the raw stored HTML here would be the one place the sanitiser is skipped.
    return {
      html: '<pre class="halcyon-plain">Message bodies are only available in the desktop app.</pre>',
      blockedRemote: 0,
      inlined: 0,
      fromPlainText: true,
    }
  }
  return invoke<Rendered>('message_body', { messageId, loadRemote })
}

/**
 * Opens a link from a message in the default browser. docs/03 §6.6.
 *
 * Returns without opening when the visible link text names a different host from the real
 * destination — the caller confirms first. `visibleText` is what the message displayed.
 */
export async function openExternal(url: string, visibleText: string): Promise<LinkOutcome> {
  if (!runningInTauri) {
    window.open(url, '_blank', 'noopener,noreferrer')
    return { opened: true, mismatch: null }
  }
  return invoke<LinkOutcome>('open_external', { url, visibleText })
}

/** Opens a link the user has confirmed despite a host mismatch. */
export async function openExternalConfirmed(mismatch: HostMismatch): Promise<void> {
  if (!runningInTauri) {
    window.open(mismatch.url, '_blank', 'noopener,noreferrer')
    return
  }
  await invoke('open_external_confirmed', { url: mismatch.url })
}

/* ------------------------------------------------------------------- attachments */

/**
 * An attachment's bytes, as a `data:` URI the previewer can show directly.
 *
 * Only ever a type the core considers previewable — an image, text, JSON or a PDF. Anything
 * else is refused there rather than here: the decision about what the app will render is a
 * security decision, and it belongs on the side of the boundary that cannot be bypassed.
 */
export async function attachmentPreview(attachmentId: number): Promise<AttachmentData> {
  if (!runningInTauri) throw new Error('Attachments are only available in the app.')
  return invoke<AttachmentData>('attachment_preview', { attachmentId })
}

/**
 * Saves an attachment, asking the user where. Resolves to `null` if they cancelled.
 *
 * There is deliberately no "open" — see `ipc/attachments.rs`. Saving puts the file where the
 * user chose, with the shell's own warnings intact when they open it themselves.
 */
export async function attachmentSave(attachmentId: number): Promise<string | null> {
  if (!runningInTauri) return null
  return invoke<string | null>('attachment_save', { attachmentId })
}

/* ---------------------------------------------------------------------- compose */

/**
 * Opens a compose window. docs/01 §6 — *a separate floating window, not a pane.*
 *
 * Returns the new window's label. Each call opens a distinct window, so several drafts can be
 * open at once and Windows treats them as separate taskbar entries — which is what a separate
 * window is for.
 */
export async function composeOpen(messageId?: number, kind?: string): Promise<string> {
  if (!runningInTauri) return ''
  return invoke<string>('compose_open', {
    messageId: messageId ?? null,
    kind: kind ?? null,
  })
}

/** Who a reply goes to, what it quotes, and how it threads. Computed by the core. */
export async function composeReply(messageId: number, kind: string): Promise<ReplyDraft> {
  if (!runningInTauri) throw new Error('Composing is only available in the app.')
  return invoke<ReplyDraft>('compose_reply', { messageId, kind })
}

/**
 * Queues a message and returns its outbox id, which is what Undo Send cancels.
 *
 * Resolves once the message is **durably on disk and in the outbox** — not once it has been
 * transmitted. The window can close immediately without risking the message, and for the length
 * of the undo hold nothing has been sent.
 */
export async function composeSend(message: OutgoingMessage): Promise<number> {
  if (!runningInTauri) throw new Error('Sending is only available in the app.')
  return invoke<number>('compose_send', { message })
}

/**
 * Cancels a queued message. This is Undo Send.
 *
 * Resolves to false when it is too late — the message is already on its way and no local action
 * recalls it. Say so rather than reporting success; the user would otherwise find out from the
 * recipient.
 */
export async function composeUndo(id: number): Promise<boolean> {
  if (!runningInTauri) return false
  return invoke<boolean>('compose_undo', { id })
}

/** Everything waiting or failed in the outbox. */
export async function outboxList(): Promise<OutboxRow[]> {
  if (!runningInTauri) return []
  return invoke<OutboxRow[]>('outbox_list')
}

/** Closes the window this code is running in. Used by compose once a message is queued. */
export async function closeThisWindow(): Promise<void> {
  if (!runningInTauri) return
  await getCurrentWindow().close()
}

/** Fires as a queued message moves through the outbox. */
export async function onOutboxProgress(
  handler: (progress: OutboxProgress) => void,
): Promise<UnlistenFn> {
  if (!runningInTauri) return () => undefined
  return listen<OutboxProgress>('outbox:progress', (event) => {
    handler(event.payload)
  })
}

export interface OutboxProgress {
  id: number
  accountId: number
  state: string
  error: string | null
}
