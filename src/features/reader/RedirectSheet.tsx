import { useState } from 'react'

import { RecipientField } from '@/features/compose/RecipientField'
import { redirectMessage } from '@/lib/ipc'
import { Button, Sheet, useToast, type Token } from '@/ui'

import styles from './RedirectSheet.module.css'

export interface RedirectSheetProps {
  open: boolean
  onOpenChange: (open: boolean) => void
  messageId: number
  subject: string
}

/** Splits `Name <addr>` or a bare address into the address alone. */
function emailOf(value: string): string {
  const open = value.lastIndexOf('<')
  const close = value.lastIndexOf('>')
  return (open >= 0 && close > open ? value.slice(open + 1, close) : value).trim()
}

function nameOf(value: string): string | null {
  const open = value.lastIndexOf('<')
  if (open < 0) return null
  const name = value.slice(0, open).trim().replace(/^"|"$/g, '')
  return name === '' ? null : name
}

function looksLikeAddress(value: string): boolean {
  const email = emailOf(value)
  return /^[^\s@]+@[^\s@.]+\.[^\s@]+$/.test(email)
}

/**
 * Redirect. docs/01 §6.
 *
 * Deliberately not a compose window. There is nothing to write: the message goes on exactly as
 * it arrived, and offering a body to type into would suggest otherwise — the recipient would
 * never see it, and the sender would have no way to find that out.
 *
 * The explanation is on screen rather than in a tooltip, because "Redirect" and "Forward" are
 * one word apart and produce very different results in the recipient's client.
 */
export function RedirectSheet({ open, onOpenChange, messageId, subject }: RedirectSheetProps) {
  const [tokens, setTokens] = useState<Token[]>([])
  const [sending, setSending] = useState(false)
  const toast = useToast()

  const send = () => {
    const usable = tokens.filter((token) => looksLikeAddress(token.value))
    if (usable.length === 0) return

    setSending(true)

    void redirectMessage(
      messageId,
      usable.map((token) => ({ name: nameOf(token.value), email: emailOf(token.value) })),
      [],
      [],
    )
      .then(() => {
        toast.show({ title: 'Redirected' })
        setTokens([])
        onOpenChange(false)
      })
      .catch((error: unknown) => {
        // The core's own words. "This message has not been downloaded in full yet" is
        // something the user can act on; "redirect failed" is not.
        toast.show({
          title: 'The message was not redirected',
          description: error instanceof Error ? error.message : String(error),
        })
      })
      .finally(() => {
        setSending(false)
      })
  }

  return (
    <Sheet
      open={open}
      onOpenChange={onOpenChange}
      title="Redirect"
      {...(subject === '' ? {} : { description: `“${subject}”` })}
      footer={
        <>
          <Button
            variant="bordered"
            onClick={() => {
              onOpenChange(false)
            }}
          >
            Cancel
          </Button>
          <Button
            variant="filled"
            disabled={sending || !tokens.some((token) => looksLikeAddress(token.value))}
            onClick={send}
          >
            Redirect
          </Button>
        </>
      }
    >
      <RecipientField
        label="To:"
        tokens={tokens}
        onTokensChange={setTokens}
        validate={looksLikeAddress}
      />

      <p className={styles.note}>
        The message is passed on exactly as it arrived, still from its original sender. A reply goes
        back to them rather than to you.
      </p>
    </Sheet>
  )
}
