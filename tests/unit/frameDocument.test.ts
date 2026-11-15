import { describe, expect, it } from 'vitest'

import { frameDocument } from '@/features/reader/frameDocument'

describe('the frame document', () => {
  it('turns off DNS prefetching before the message is parsed', () => {
    // L34 in the 2026-11-14 audit. A host looked up as the message loads tells its owner the
    // message was opened, remote images or not, and the CSP does not cover lookups.
    const document = frameDocument('<a href="https://per-recipient.example.test/">x</a>', false, '')
    const meta = '<meta http-equiv="x-dns-prefetch-control" content="off">'

    expect(document).toContain(meta)
    expect(document.indexOf(meta)).toBeLessThan(document.indexOf('<body'))
  })
})
