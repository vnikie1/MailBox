import { render, waitFor } from '@testing-library/react'
import { describe, expect, it, vi } from 'vitest'

import { Editor } from '@/features/compose/Editor'

/**
 * The compose editor, loading markup it wrote itself.
 *
 * M47 in the 2026-11-14 audit: Lexical's importers keep bold and italic and drop every other
 * style on a span, so a signature written in colour at a larger size came back plain each time it
 * was loaded — into a new message, and into the Signatures pane, which then saved the plain one
 * back over it.
 */

const STYLED = '<p><span style="color: rgb(224, 62, 45); font-size: 18px;">Ada Lovelace</span></p>'

describe('the compose editor', () => {
  it('keeps the colour and size the format bar can set when it loads markup', async () => {
    const onChange = vi.fn<(html: string, text: string) => void>()
    render(<Editor initialHtml={STYLED} onChange={onChange} ariaLabel="Body" />)

    await waitFor(() => {
      expect(onChange).toHaveBeenCalled()
    })

    const html = onChange.mock.calls.at(-1)?.[0] ?? ''
    expect(html).toContain('Ada Lovelace')
    expect(html).toMatch(/color: rgb\(224, 62, 45\)/)
    expect(html).toMatch(/font-size: 18px/)
  })

  it('replaces the signature where it stands when it changes, and nothing else', async () => {
    // M46: changing From kept the first account's signature, because the editor took its content
    // once and nothing could reach the signature in it afterwards.
    const onChange = vi.fn<(html: string, text: string, origin: string) => void>()
    const content = { before: '<p>Typed first</p>', after: '<blockquote>The original</blockquote>' }
    const props = { initialContent: content, onChange, ariaLabel: 'Body' }

    const { rerender } = render(
      <Editor {...props} signature={{ html: '<p>Ada, Work</p>', placement: 'above' }} />,
    )
    await waitFor(() => {
      expect(onChange.mock.calls.at(-1)?.[1]).toContain('Ada, Work')
    })

    rerender(<Editor {...props} signature={{ html: '<p>Ada, Home</p>', placement: 'above' }} />)
    await waitFor(() => {
      expect(onChange.mock.calls.at(-1)?.[1]).toContain('Ada, Home')
    })

    const [, text, origin] = onChange.mock.calls.at(-1) ?? []
    expect(text).not.toContain('Ada, Work')
    // In the same place: after what was typed, before the quote.
    expect(text?.indexOf('Typed first')).toBeLessThan(text?.indexOf('Ada, Home') ?? -1)
    expect(text?.indexOf('Ada, Home')).toBeLessThan(text?.indexOf('The original') ?? -1)
    expect(origin).toBe('load')
  })

  it('puts a signature in when the first account had none', async () => {
    const onChange = vi.fn<(html: string, text: string, origin: string) => void>()
    const props = {
      initialContent: { before: '', after: '<blockquote>The original</blockquote>' },
      onChange,
      ariaLabel: 'Body',
    }

    const { rerender } = render(<Editor {...props} signature={{ html: '', placement: 'above' }} />)
    await waitFor(() => {
      expect(onChange).toHaveBeenCalled()
    })

    rerender(<Editor {...props} signature={{ html: '<p>Ada, Home</p>', placement: 'above' }} />)
    await waitFor(() => {
      expect(onChange.mock.calls.at(-1)?.[1]).toContain('Ada, Home')
    })

    const text = onChange.mock.calls.at(-1)?.[1] ?? ''
    expect(text.indexOf('Ada, Home')).toBeLessThan(text.indexOf('The original'))
  })

  it('says the change it reports for loading is its own, not the user’s', async () => {
    const onChange = vi.fn<(html: string, text: string, origin?: string) => void>()
    render(<Editor initialHtml={STYLED} onChange={onChange} ariaLabel="Body" />)

    await waitFor(() => {
      expect(onChange).toHaveBeenCalled()
    })

    expect(onChange.mock.calls.map((call) => call[2])).toEqual(['load'])
  })
})
