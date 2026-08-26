import { useState } from 'react'
import { User } from 'lucide-react'

import { cx } from '@/lib/cx'
import { tintIndexFor } from '@/lib/tint'

import styles from './Avatar.module.css'

export interface AvatarProps {
  name?: string
  email?: string
  src?: string
  size?: 'sm' | 'md' | 'lg'
  className?: string | undefined
}

/**
 * Initials from a display name — "Ada Lovelace" to "AL", "Ada" to "A".
 *
 * Graphemes, not characters. `charAt(0)` returns half a surrogate pair for an emoji or an
 * astral-plane initial, and even a code-point split decomposes "é" written as e + combining
 * accent, or a flag, into pieces. `Intl.Segmenter` is the only thing that gets a real
 * contact list right, and display names in mail are exactly as unruly as they sound.
 */
const graphemes = new Intl.Segmenter(undefined, { granularity: 'grapheme' })

function firstGrapheme(word: string): string {
  for (const { segment } of graphemes.segment(word)) return segment
  return ''
}

function initialsOf(name: string): string {
  const words = name
    .trim()
    .split(/\s+/)
    .filter((word) => word.length > 0)

  const first = words[0]
  if (!first) return ''

  const last = words.length > 1 ? words[words.length - 1] : undefined
  return (firstGrapheme(first) + (last ? firstGrapheme(last) : '')).toUpperCase()
}

/**
 * A contact avatar. docs/02 §6.4 (reader header, 32), §6.7 (recipient chip, 16).
 *
 * Tinted per contact, from a hash of the address — docs/01 §4. The tints are muted rather
 * than saturated, which is both what assets/reference/ shows Mail doing and what keeps this
 * inside standing rule 2: a wall of *saturated* initials circles is the loudest way to lose
 * the restraint docs/01 §9.3 describes, but soft lavender and blue-grey discs are not that.
 *
 * A broken image URL falls back to initials rather than to a broken-image glyph, because
 * in Phase 6 these URLs come from message content and are hostile by default.
 */
export function Avatar({ name, email, src, size = 'lg', className }: AvatarProps) {
  const [failed, setFailed] = useState(false)

  const initials = name ? initialsOf(name) : email ? firstGrapheme(email.trim()).toUpperCase() : ''
  const label = name ?? email ?? ''
  const tint = tintIndexFor(email ?? name ?? '')

  return (
    <span
      className={cx(styles.avatar, styles[size], className)}
      role="img"
      aria-label={label || 'Unknown sender'}
      data-tint={tint}
    >
      {src && !failed ? (
        <img
          className={styles.image}
          src={src}
          alt=""
          onError={() => {
            setFailed(true)
          }}
        />
      ) : initials ? (
        <span aria-hidden="true">{initials}</span>
      ) : (
        <User className={styles.glyph} aria-hidden="true" strokeWidth={1.5} />
      )}
    </span>
  )
}
