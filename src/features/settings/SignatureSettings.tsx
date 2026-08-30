import { useEffect, useRef, useState } from 'react'

import { useAccounts } from '@/app/queries'
import { Editor } from '@/features/compose/Editor'
import type { Signature } from '@/lib/generated/Signature'
import { signatureGet, signatureSet } from '@/lib/ipc'

import styles from './settings.module.css'
import pane from './SignatureSettings.module.css'

/**
 * Settings → Signatures. docs/01 §6, docs/06 Phase 11.
 *
 * The core has had `signature_get` and `signature_set` since Phase 7, with tests, sanitising,
 * and the placement rule that decides whether the signature sits above or below a quoted
 * reply — and until now **nothing called either of them.** Every message this app has ever sent
 * went out unsigned, not because signatures were unimplemented but because there was no way to
 * type one. That is the failure mode a settings window with no Signatures pane produces, and it
 * is invisible from the Rust side: the tests pass, the column exists, the feature is absent.
 *
 * One signature per account rather than a named list. Mail allows several and assigns them per
 * account; the column here holds one, and inventing a second table to hold more before anybody
 * has asked for it would be the wrong order to do the work in.
 */

/** How long after the last keystroke the signature is stored. */
const SAVE_AFTER_MS = 800

type Status = 'loading' | 'ready' | 'saving' | 'saved'

export function SignatureSettings() {
  const { data: accounts = [] } = useAccounts()
  const [accountId, setAccountId] = useState<number | null>(null)
  const [signature, setSignature] = useState<Signature | null>(null)
  const [status, setStatus] = useState<Status>('loading')

  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  /**
   * What is in the editor now.
   *
   * A ref rather than state, and this is the whole reason the component is shaped this way.
   * `signature.html` feeds the editor's `initialHtml`, and putting the editor's own output back
   * into that prop corrupts the text being typed: an empty signature means the editor's one-shot
   * loader has not fired yet, so the first keystroke comes straight back in as "initial" content
   * and lands where the loader puts it. Typing "Vishal Singh" produced "ishal SinghV". Nothing
   * the editor emits is allowed back into a prop the editor reads.
   */
  const html = useRef('')

  // The first account, once they arrive. Not a default in `useState`, because at that point
  // the list is empty and the pane would sit on "no account" for ever.
  useEffect(() => {
    const first = accounts[0]
    if (accountId === null && first !== undefined) setAccountId(first.id)
  }, [accounts, accountId])

  useEffect(() => {
    if (accountId === null) return
    let live = true

    setStatus('loading')
    setSignature(null)

    void signatureGet(accountId).then((value) => {
      if (!live) return
      html.current = value.html
      setSignature(value)
      setStatus('ready')
    })

    return () => {
      live = false
    }
  }, [accountId])

  // Cleared on unmount, so a pending save cannot fire against an account the pane has left.
  useEffect(
    () => () => {
      if (timer.current !== null) clearTimeout(timer.current)
    },
    [],
  )

  const store = (next: Signature) => {
    if (accountId === null) return

    html.current = next.html
    // Only the placement is state: it is a control the user reads back, and unlike the body it
    // is never written by the editor.
    setSignature((current) =>
      current === null ? current : { ...current, placement: next.placement },
    )
    setStatus('saving')

    if (timer.current !== null) clearTimeout(timer.current)
    timer.current = setTimeout(() => {
      void signatureSet(accountId, next.html, next.placement).then(() => {
        setStatus('saved')
      })
    }, SAVE_AFTER_MS)
  }

  if (accounts.length === 0) {
    return (
      <section className={styles.section}>
        <h3 className={styles.heading}>Signatures</h3>
        <p className={styles.hint}>
          A signature belongs to an account. Add one under Accounts and it will appear here.
        </p>
      </section>
    )
  }

  return (
    <section className={styles.section}>
      <h3 className={styles.heading}>Signatures</h3>

      <label className={styles.row}>
        <span className={styles.name}>Account</span>
        <select
          className={pane.picker}
          value={accountId ?? ''}
          onChange={(event) => {
            setAccountId(Number(event.target.value))
          }}
        >
          {accounts.map((account) => (
            <option key={account.id} value={account.id}>
              {account.email}
            </option>
          ))}
        </select>
      </label>

      {signature === null ? (
        <p className={styles.hint}>Loading…</p>
      ) : (
        <div className={pane.editor}>
          {/* Keyed by account. The editor takes its initial HTML once and then owns its own
              state — without the key, switching account would leave the previous account's
              signature on screen and save it over the new one on the next keystroke. */}
          <Editor
            key={accountId ?? 'none'}
            initialHtml={signature.html}
            ariaLabel="Signature"
            onChange={(next) => {
              store({ ...signature, html: next })
            }}
          />
        </div>
      )}

      <fieldset className={styles.group}>
        <legend className={styles.legend}>In a reply, place the signature</legend>

        {['above', 'below'].map((placement) => (
          <label key={placement} className={styles.choice}>
            <input
              type="radio"
              name="placement"
              className={styles.radio}
              checked={signature?.placement === placement}
              disabled={signature === null}
              onChange={() => {
                if (signature !== null) store({ html: html.current, placement })
              }}
            />
            {placement === 'above' ? 'Above the quoted text' : 'Below the quoted text'}
          </label>
        ))}
      </fieldset>

      <p className={styles.hint} aria-live="polite">
        {status === 'saving'
          ? 'Saving…'
          : status === 'saved'
            ? 'Saved. It will be added to new messages from this account.'
            : 'Added to the bottom of every message sent from this account.'}
      </p>
    </section>
  )
}
