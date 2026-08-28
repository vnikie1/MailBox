import { readFileSync, readdirSync, statSync } from 'node:fs'
import { join } from 'node:path'

import { describe, expect, it } from 'vitest'

/**
 * The accessibility pass, as tests. docs/06 Phase 10.
 *
 * docs/06 asks for "Narrator can read and act on the list; focus order; contrast; reduced
 * motion; reduce transparency; 200% text scaling". Some of that only a person with a screen
 * reader can judge, and the gate asks for a recorded walkthrough for exactly that reason.
 *
 * What *can* be checked mechanically is the set of mistakes that silently undo the rest: a
 * control with no accessible name, a focus ring removed and not replaced, a fixed pixel font
 * size that ignores the system text setting. Each of those is invisible in review and obvious
 * to somebody relying on it.
 */

function filesUnder(directory: string, extension: string): string[] {
  const found: string[] = []

  for (const entry of readdirSync(directory)) {
    const path = join(directory, entry)

    if (statSync(path).isDirectory()) {
      found.push(...filesUnder(path, extension))
      continue
    }

    if (entry.endsWith(extension)) found.push(path)
  }

  return found
}

const CSS = filesUnder('src', '.css')
const TSX = filesUnder('src', '.tsx')

describe('the audit itself', () => {
  it('finds files to audit', () => {
    // Guards everything below: a wrong path would make every check pass over an empty list.
    expect(CSS.length).toBeGreaterThan(10)
    expect(TSX.length).toBeGreaterThan(10)
  })
})

describe('focus', () => {
  it('never removes a focus ring without replacing it', () => {
    // `outline: none` with nothing in its place is the single most common way an app becomes
    // unusable by keyboard: every control still works and none of them shows where you are.
    for (const file of CSS) {
      const css = readFileSync(file, 'utf8')

      for (const block of css.split('}')) {
        if (!/outline:\s*none/.test(block)) continue

        // The exemption has to be written into the stylesheet to claim it. Some indicators
        // legitimately live somewhere other than the element that holds focus — a caret in a
        // text surface, the selected row of a list, the highlighted item of a menu — and in
        // each case a ring would be worse. Requiring the marker in the CSS keeps the reason
        // in front of whoever reads that file next, which an allow-list here would not.
        if (block.includes('focus-ring-exempt')) continue

        const replaced = /box-shadow|outline-color|outline:\s*var\(/.test(block)
        expect(replaced, `${file} removes a focus ring without replacing it:\n${block}`).toBe(true)
      }
    }
  })
})

describe('text scaling', () => {
  it('sizes type from tokens rather than fixed pixels', () => {
    // Windows' text-size setting scales rem. A font-size in px ignores it, so a user at 200%
    // gets a layout that grew around text that did not.
    for (const file of CSS) {
      if (file.includes('tokens')) continue

      const css = readFileSync(file, 'utf8')
      const offenders = [...css.matchAll(/font-size:\s*([^;]+);/g)]
        .map((match) => match[1]?.trim() ?? '')
        .filter((value) => /\d+px/.test(value))

      expect(offenders, `${file} sets a fixed font size`).toEqual([])
    }
  })
})

/**
 * Every `<IconButton …>` opening tag in a file.
 *
 * A regex cannot do this, and the obvious one is actively misleading. `<IconButton\b[^>]*?/>`
 * stops at the first `>` — which in JSX is usually the arrow of an inline `onClick={() => …}`,
 * not the end of the tag. Written that way this check silently skipped 26 of the app's 56
 * IconButtons and still passed, which is worse than not having the check at all.
 *
 * So: scan forward from the tag name and only treat `>` as the end when no brace is open.
 */
function iconButtonTags(source: string): string[] {
  const tags: string[] = []

  for (const match of source.matchAll(/<IconButton\b/g)) {
    const start = match.index
    let depth = 0

    for (let at = start; at < source.length; at += 1) {
      const character = source[at]

      if (character === '{') depth += 1
      else if (character === '}') depth -= 1
      else if (character === '>' && depth === 0) {
        tags.push(source.slice(start, at + 1))
        break
      }
    }
  }

  return tags
}

describe('names', () => {
  it('gives every icon-only button an accessible name', () => {
    // An IconButton renders a glyph and nothing else, so without a label Narrator announces
    // "button" and the user has to press it to find out what it does.
    let checked = 0

    for (const file of TSX) {
      for (const tag of iconButtonTags(readFileSync(file, 'utf8'))) {
        checked += 1

        const named = /\blabel=|\baria-label=|\baria-labelledby=/.test(tag)
        expect(named, `${file} has an IconButton with no name:\n${tag}`).toBe(true)
      }
    }

    // Asserted because the failure this check actually had was finding almost nothing and
    // passing. If a refactor renames the component, this notices instead of going quiet.
    expect(checked).toBeGreaterThan(50)
  })

  it('hides decorative glyphs from the reader', () => {
    // A glyph beside a label that is also announced doubles every row: "envelope, Inbox".
    expect(readFileSync('src/ui/EmptyState.tsx', 'utf8')).toContain('aria-hidden="true"')
  })
})

describe('transparency and motion', () => {
  it('can turn the materials off from one place', () => {
    // Reduce Transparency is a system setting, and honouring it per component would mean every
    // new surface has to remember. The materials are roles defined in one file.
    const semantic = readFileSync('src/styles/tokens/semantic.css', 'utf8')

    expect(semantic).toContain('--filter-sidebar')
    expect(semantic).toContain('--filter-menu')
  })

  it('collapses motion from one place', () => {
    expect(readFileSync('src/styles/tokens/primitive.css', 'utf8')).toContain(
      'prefers-reduced-motion',
    )
  })
})
