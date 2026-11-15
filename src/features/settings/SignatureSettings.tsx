import { useEffect, useRef, useState } from 'react'

import { useAccounts } from '@/app/queries'
import { Editor } from '@/features/compose/Editor'
import type { Signature } from '@/lib/generated/Signature'
import { signatureGet, signatureSet } from '@/lib/ipc'
import { Segmented, Select, type SegmentedOption } from '@/ui'

import { Field, Form, FullRow } from './SettingsForm'
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

/** The two halves of `Signature['placement']`, in the order a reply is read. */
const PLACEMENTS: SegmentedOption<Signature['placement']>[] = [
  { value: 'above', label: 'Above the quote' },
  { value: 'below', label: 'Below the quote' },
]

type Status = 'loading' | 'ready' | 'saving' | 'saved'

export function SignatureSettings() {
  const { data: accounts = [] } = useAccounts()
  const [accountId, setAccountId] = useState<number | null>(null)
  const [status, setStatus] = useState<Status>('loading')

  /**
   * The signature as loaded, **with the account it belongs to**.
   *
   * It used to be a bare `Signature`, cleared by an effect when the account changed — and an
   * effect runs after the render. That one render showed the new account's editor holding the
   * previous account's signature; the editor reports what it loads as a change; and 800ms later
   * the pane saved it to the new account, over whatever was there (H18, 2026-11-14 audit). Read
   * through the account, a value loaded for another one is simply not there, in the same render.
   */
  const [loaded, setLoaded] = useState<{ accountId: number; value: Signature } | null>(null)
  const signature = loaded !== null && loaded.accountId === accountId ? loaded.value : null
  const setSignature = (update: (current: Signature) => Signature) => {
    setLoaded((current) =>
      current === null ? current : { ...current, value: update(current.value) },
    )
  }

  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  /** The save the timer is waiting to make, with the account it belongs to. */
  const pending = useRef<{ accountId: number; signature: Signature } | null>(null)

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

    void signatureGet(accountId).then((value) => {
      if (!live) return
      html.current = value.html
      setLoaded({ accountId, value })
      setStatus('ready')
    })

    return () => {
      live = false
    }
  }, [accountId])

  /**
   * Made at once when the pane goes away, not dropped.
   *
   * The timer used to be cleared on unmount, so the last 0.8s of typing — the end of the
   * signature, usually — was lost by switching to another pane before it fired (L24, 2026-11-14
   * audit). The save carries the account it was typed for, so making it now cannot put it on
   * another one: what H18 fixed was the pane loading one account's signature into another's
   * editor, not a pending save.
   */
  useEffect(
    () => () => {
      if (timer.current !== null) clearTimeout(timer.current)

      const unsaved = pending.current
      pending.current = null
      if (unsaved !== null) {
        void signatureSet(unsaved.accountId, unsaved.signature.html, unsaved.signature.placement)
      }
    },
    [],
  )

  const store = (next: Signature) => {
    if (accountId === null) return

    html.current = next.html
    // Only the placement is state: it is a control the user reads back, and unlike the body it
    // is never written by the editor.
    setSignature((current) => ({ ...current, placement: next.placement }))
    setStatus('saving')

    const save = { accountId, signature: next }
    pending.current = save

    if (timer.current !== null) clearTimeout(timer.current)
    timer.current = setTimeout(() => {
      if (pending.current === save) pending.current = null
      void signatureSet(save.accountId, next.html, next.placement).then(() => {
        setStatus('saved')
      })
    }, SAVE_AFTER_MS)
  }

  if (accounts.length === 0) {
    return (
      <section className={styles.section}>
        <h2 className={styles.heading}>Signature</h2>
        <p className={styles.hint}>
          A signature belongs to an account. Add one under Accounts and it will appear here.
        </p>
      </section>
    )
  }

  return (
    <section className={styles.section}>
      <h2 className={styles.heading}>Signature</h2>

      <Form>
        <Field label="Account" htmlFor="signature-account">
          <Select
            id="signature-account"
            label="Account"
            hideLabel
            className={pane.picker}
            options={accounts.map((account) => ({ value: account.id, label: account.email }))}
            value={accountId}
            onValueChange={setAccountId}
          />
        </Field>

        {/* The editor takes the full width of the pane — see `.editor` for why it is not
            indented to the control column like everything else here. */}
        <FullRow>
          {signature === null ? (
            <p className={styles.hint}>Loading…</p>
          ) : (
            <div className={pane.editor}>
              {/* Keyed by account. The editor takes its initial HTML once and then owns its
                  own state — without the key, switching account would leave the previous
                  account's signature on screen and save it over the new one on the next
                  keystroke. */}
              <Editor
                key={accountId ?? 'none'}
                initialHtml={signature.html}
                ariaLabel="Signature"
                onChange={(next, _text, origin) => {
                  // The signature as it loaded is what is already stored. Saving it back was
                  // harmless only while loading was lossless, and it was not (M47).
                  if (origin === 'load') {
                    html.current = next
                    return
                  }
                  store({ ...signature, html: next })
                }}
              />
            </div>
          )}
        </FullRow>

        <Field label="In a reply" labelId="placement-label">
          <Segmented
            label="In a reply, place the signature"
            labelledBy="placement-label"
            options={PLACEMENTS}
            value={signature?.placement ?? null}
            disabled={signature === null}
            onValueChange={(placement) => {
              if (signature !== null) store({ html: html.current, placement })
            }}
          />

          {/* Height reserved, so the pane does not jog as it saves. Standing rule 6. */}
          <p className={styles.status} aria-live="polite">
            {status === 'saving'
              ? 'Saving…'
              : status === 'saved'
                ? 'Saved. It will be added to new messages from this account.'
                : 'Added to the bottom of every message sent from this account.'}
          </p>
        </Field>
      </Form>
    </section>
  )
}
