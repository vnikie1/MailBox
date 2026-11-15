import { render, screen, waitFor } from '@testing-library/react'
import userEvent from '@testing-library/user-event'
import { useState } from 'react'
import { afterEach, describe, expect, it, vi } from 'vitest'

import type { Suggestion } from '@/lib/generated/Suggestion'

/**
 * Leaving a search from the search field.
 *
 * L43 in the 2026-11-14 audit: Escape closed the suggestions and did nothing else, where in Mail
 * it clears the search. And the field's clear button never existed: it was passed to
 * `TextField` under a prop the field does not have.
 */

let suggestions: Suggestion[] = []

vi.mock('@/lib/search', () => ({
  rememberSearch: () => Promise.resolve(),
  searchHistory: () => Promise.resolve([]),
  suggestSearch: () => Promise.resolve(suggestions),
}))

afterEach(() => {
  suggestions = []
})

const committed = vi.fn<(text: string) => void>()

async function searchFor(text: string) {
  const { SearchField } = await import('@/features/search/SearchField')

  function Harness() {
    const [value, setValue] = useState(text)
    return <SearchField value={value} onChange={setValue} onCommit={committed} />
  }

  render(<Harness />)
  const field = screen.getByRole('textbox', { name: 'Search' })
  await userEvent.click(field)
  return field
}

describe('the search field', () => {
  it('clears the search on Escape', async () => {
    const field = await searchFor('figures')

    await userEvent.keyboard('{Escape}')

    expect(field).toHaveValue('')
    expect(committed).toHaveBeenLastCalledWith('')
  })

  it('closes the suggestions first, when they are up', async () => {
    suggestions = [{ kind: 'text', label: 'figures', insert: 'figures', detail: null }]
    const field = await searchFor('figures')
    await userEvent.type(field, 's')
    expect(await screen.findByRole('listbox', { name: 'Search suggestions' })).toBeTruthy()

    await userEvent.keyboard('{Escape}')
    expect(screen.queryByRole('listbox', { name: 'Search suggestions' })).toBeNull()
    expect(field).toHaveValue('figuress')

    await userEvent.keyboard('{Escape}')
    await waitFor(() => {
      expect(field).toHaveValue('')
    })
  })

  it('has a button that clears it', async () => {
    const field = await searchFor('figures')

    await userEvent.click(screen.getByRole('button', { name: 'Clear search' }))

    expect(field).toHaveValue('')
    expect(committed).toHaveBeenLastCalledWith('')
  })
})
