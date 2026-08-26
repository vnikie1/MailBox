import { useState } from 'react'
import { describe, expect, it } from 'vitest'
import { render, screen } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import { TokenField, type Token } from '@/ui'

/**
 * TokenField keyboard behaviour. Phase 1 exit gate.
 *
 * The behaviour that matters most is the two-step delete: Backspace on an empty field
 * *selects* the last recipient and only a second press removes it. A compose window is
 * one Tab away from Send, and a field where one stray keystroke silently drops a
 * recipient is how mail goes to the wrong people.
 */

const INITIAL: Token[] = [
  { id: 'a', label: 'Ada Lovelace', value: 'ada@example.com' },
  { id: 'b', label: 'Grace Hopper', value: 'grace@example.com' },
]

function Harness({ initial = INITIAL }: { initial?: Token[] }) {
  const [tokens, setTokens] = useState<Token[]>(initial)
  return (
    <TokenField
      label="To:"
      tokens={tokens}
      onTokensChange={setTokens}
      placeholder="Add a recipient"
      validate={(value) => value.includes('@')}
    />
  )
}

function field() {
  return screen.getByRole('textbox')
}

function selectedChip(): HTMLElement | null {
  return document.querySelector<HTMLElement>('[aria-current="true"]')
}

function chipLabels(): string[] {
  return screen.getAllByRole('listitem').map((item) => item.textContent.replace(/Remove.*/, ''))
}

describe('TokenField', () => {
  it('commits what you type when you press Enter', async () => {
    const user = userEvent.setup()
    render(<Harness initial={[]} />)

    await user.click(field())
    await user.keyboard('ada@example.com{Enter}')

    expect(chipLabels()).toEqual(['ada@example.com'])
    expect(field()).toHaveValue('')
  })

  it('commits on a comma or a semicolon without leaving the separator in the token', async () => {
    const user = userEvent.setup()
    render(<Harness initial={[]} />)

    await user.click(field())
    await user.keyboard('ada@example.com,grace@example.com;')

    expect(chipLabels()).toEqual(['ada@example.com', 'grace@example.com'])
  })

  it('marks a token that does not validate rather than refusing it', async () => {
    const user = userEvent.setup()
    render(<Harness initial={[]} />)

    await user.click(field())
    await user.keyboard('not an address{Enter}')

    // Refusing it silently would lose what the user typed. It is shown, and shown wrong.
    expect(chipLabels()).toEqual(['not an address'])
  })

  it('selects the last token on Backspace and only removes it on the second press', async () => {
    const user = userEvent.setup()
    render(<Harness />)

    await user.click(field())
    await user.keyboard('{Backspace}')

    expect(selectedChip()).toHaveTextContent('Grace Hopper')
    expect(chipLabels()).toHaveLength(2)

    await user.keyboard('{Backspace}')
    expect(chipLabels()).toEqual(['Ada Lovelace'])
  })

  it('does not touch the tokens while there is text to delete', async () => {
    const user = userEvent.setup()
    render(<Harness />)

    await user.click(field())
    await user.keyboard('ab{Backspace}{Backspace}')

    expect(chipLabels()).toHaveLength(2)
    expect(field()).toHaveValue('')
  })

  it('walks the tokens with the arrow keys and returns to the field', async () => {
    const user = userEvent.setup()
    render(<Harness />)

    await user.click(field())
    await user.keyboard('{ArrowLeft}')
    expect(selectedChip()).toHaveTextContent('Grace Hopper')

    await user.keyboard('{ArrowLeft}')
    expect(selectedChip()).toHaveTextContent('Ada Lovelace')

    await user.keyboard('{ArrowRight}')
    expect(selectedChip()).toHaveTextContent('Grace Hopper')

    await user.keyboard('{ArrowRight}')
    expect(field()).toHaveFocus()
  })

  it('clears the selection on Escape without removing anything', async () => {
    const user = userEvent.setup()
    render(<Harness />)

    await user.click(field())
    await user.keyboard('{Backspace}{Escape}')

    expect(chipLabels()).toHaveLength(2)
    expect(selectedChip()).toBeNull()
  })

  it('splits a pasted address list into one token each', async () => {
    const user = userEvent.setup()
    render(<Harness initial={[]} />)

    await user.click(field())
    await user.paste('ada@example.com, grace@example.com; katherine@example.com')

    expect(chipLabels()).toEqual(['ada@example.com', 'grace@example.com', 'katherine@example.com'])
  })

  it('commits the half-typed address when focus leaves, rather than dropping it', async () => {
    const user = userEvent.setup()
    render(<Harness initial={[]} />)

    await user.click(field())
    await user.keyboard('ada@example.com')
    await user.tab()

    expect(chipLabels()).toEqual(['ada@example.com'])
  })

  it('is labelled as a group and tells a screen reader how to delete', () => {
    render(<Harness />)

    expect(screen.getByRole('group', { name: 'To:' })).toBeInTheDocument()
    expect(field()).toHaveAccessibleDescription(/Press Backspace to select the previous recipient/)
  })
})
