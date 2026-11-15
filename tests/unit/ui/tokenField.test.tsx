import { useState } from 'react'
import { afterEach, beforeEach, describe, expect, it } from 'vitest'
import { act, fireEvent, render, screen, within } from '@testing-library/react'
import userEvent from '@testing-library/user-event'

import { TokenField, type Token } from '@/ui'
import { readRecipient } from '@/features/compose/address'

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

function Harness({
  initial = INITIAL,
  recipients = false,
}: {
  initial?: Token[]
  /** Reads values as the compose window does: by display name, and the same by address. */
  recipients?: boolean
}) {
  const [tokens, setTokens] = useState<Token[]>(initial)
  return (
    <TokenField
      label="To:"
      tokens={tokens}
      onTokensChange={setTokens}
      placeholder="Add a recipient"
      validate={(value) => value.includes('@')}
      {...(recipients ? { read: readRecipient } : {})}
      dragGroup="recipients"
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

  it('does not split a pasted name at the comma inside its quotes', async () => {
    // M44 in the 2026-11-14 audit: a red `"Smith` chip and a `John" <john@example.com>` one.
    const user = userEvent.setup()
    render(<Harness initial={[]} recipients />)

    await user.click(field())
    await user.paste('"Smith, John" <john@example.com>, ada@example.com')

    expect(chipLabels()).toEqual(['Smith, John', 'ada@example.com'])
  })

  it('takes a comma typed inside quotes as part of the name', async () => {
    const user = userEvent.setup()
    render(<Harness initial={[]} recipients />)

    await user.click(field())
    await user.keyboard('"Smith, John" <john@example.com>{Enter}')

    expect(chipLabels()).toEqual(['Smith, John'])
  })

  it('splits text that arrived without keystrokes when Enter commits it', async () => {
    // Found driving the release build after batch 8: text put in the field whole — dropped in,
    // dictated, or set by an input method — reaches Enter with its commas, and was committed as
    // one chip holding every address. Cut where a paste is cut; the quoted comma stays.
    const user = userEvent.setup()
    render(<Harness initial={[]} recipients />)

    await user.click(field())
    fireEvent.change(field(), {
      target: { value: '"Smith, John" <john@example.com>, ada@example.com; grace@example.com' },
    })
    await user.keyboard('{Enter}')

    expect(chipLabels()).toEqual(['Smith, John', 'ada@example.com', 'grace@example.com'])
    expect(field()).toHaveValue('')
  })

  it('shows a recipient by name, not as the text it was committed as', async () => {
    // M44: a suggestion is committed as `Name <address>`, and the chip showed exactly that.
    const user = userEvent.setup()
    render(<Harness initial={[]} recipients />)

    await user.click(field())
    await user.keyboard('Ada Lovelace <ada@example.com>{Enter}')

    expect(chipLabels()).toEqual(['Ada Lovelace'])
  })

  it('does not add a recipient who is already there', async () => {
    // L21: two chips for one address, and the message went to it twice.
    const user = userEvent.setup()
    render(<Harness recipients />)

    await user.click(field())
    await user.keyboard('ADA@example.com{Enter}')
    await user.paste('grace@example.com, Grace <grace@example.com>, katherine@example.com')

    expect(chipLabels()).toEqual(['Ada Lovelace', 'Grace Hopper', 'katherine@example.com'])
    expect(field()).toHaveValue('')
  })

  it('is labelled as a group and tells a screen reader how to delete', () => {
    render(<Harness />)

    expect(screen.getByRole('group', { name: 'To:' })).toBeInTheDocument()
    expect(field()).toHaveAccessibleDescription(/Press Backspace to select the previous recipient/)
  })
})

/**
 * Moving a chip to another field, by pointer.
 *
 * M48 in the 2026-11-14 audit. This was HTML5 drag and drop, which cannot work in the compose
 * window: Tauri's native drop handler, kept on there so files dragged in from Explorer arrive
 * with their paths, takes every HTML5 drag in the page. And a chip dropped back on its own
 * field was deleted. Pointer events, as the real app sees them; jsdom has no layout, so the
 * element under the pointer is stood in for.
 */
describe('moving a chip between fields', () => {
  let under: Element | null = null

  // jsdom has neither. Defined as own properties here, and deleted after, so whatever the
  // prototypes have is what the next test sees.
  beforeEach(() => {
    Object.defineProperty(document, 'elementFromPoint', { configurable: true, value: () => under })
    Object.defineProperty(HTMLDivElement.prototype, 'setPointerCapture', {
      configurable: true,
      value: () => undefined,
    })
  })

  afterEach(() => {
    Reflect.deleteProperty(document, 'elementFromPoint')
    Reflect.deleteProperty(HTMLDivElement.prototype, 'setPointerCapture')
    under = null
  })

  function TwoFields({ cc: initialCc = [] }: { cc?: Token[] }) {
    const [to, setTo] = useState<Token[]>(INITIAL)
    const [cc, setCc] = useState<Token[]>(initialCc)
    return (
      <>
        <TokenField
          label="To:"
          tokens={to}
          onTokensChange={setTo}
          read={readRecipient}
          dragGroup="recipients"
        />
        <TokenField
          label="Cc:"
          tokens={cc}
          onTokensChange={setCc}
          read={readRecipient}
          dragGroup="recipients"
        />
      </>
    )
  }

  function chipsIn(name: string): string[] {
    return within(screen.getByRole('group', { name }))
      .queryAllByRole('listitem')
      .map((item) => item.textContent.replace(/Remove.*/, ''))
  }

  /** Where a field takes a chip: the part of it under the pointer. */
  function fieldBody(name: string): Element | null {
    return screen.getByRole('group', { name }).querySelector('[data-token-field]')
  }

  function pointer(target: Element, type: string, clientX: number) {
    const event = new Event(type, { bubbles: true, cancelable: true })
    Object.assign(event, { clientX, clientY: 10, pointerId: 1, button: 0, pointerType: 'mouse' })
    act(() => {
      target.dispatchEvent(event)
    })
  }

  /** Presses on `chip`, moves `distance` across to wherever `over` is, and lets go. */
  function drag(chip: Element, over: Element | null, distance = 40) {
    pointer(chip, 'pointerdown', 10)
    under = over
    pointer(chip, 'pointermove', 10 + distance / 2)
    pointer(chip, 'pointermove', 10 + distance)
    pointer(chip, 'pointerup', 10 + distance)
  }

  it('moves a chip to the field it is dropped on', () => {
    render(<TwoFields />)

    drag(screen.getByText('Ada Lovelace'), fieldBody('Cc:'))

    expect(chipsIn('To:')).toEqual(['Grace Hopper'])
    expect(chipsIn('Cc:')).toEqual(['Ada Lovelace'])
  })

  it('keeps a chip dropped back on its own field', () => {
    render(<TwoFields />)

    drag(screen.getByText('Ada Lovelace'), fieldBody('To:'))

    expect(chipsIn('To:')).toEqual(['Ada Lovelace', 'Grace Hopper'])
    expect(chipsIn('Cc:')).toEqual([])
  })

  it('keeps a chip let go of over nothing that takes it', () => {
    render(<TwoFields />)

    drag(screen.getByText('Ada Lovelace'), document.body)

    expect(chipsIn('To:')).toEqual(['Ada Lovelace', 'Grace Hopper'])
  })

  it('does not take a press that hardly moves for a drag', () => {
    render(<TwoFields />)

    drag(screen.getByText('Ada Lovelace'), fieldBody('Cc:'), 2)

    expect(chipsIn('To:')).toEqual(['Ada Lovelace', 'Grace Hopper'])
    expect(chipsIn('Cc:')).toEqual([])
  })

  it('does not make a second chip in a field that has the address already', () => {
    render(<TwoFields cc={[{ id: 'c', label: 'Ada', value: 'Ada <ADA@example.com>' }]} />)

    drag(screen.getByText('Ada Lovelace'), fieldBody('Cc:'))

    expect(chipsIn('To:')).toEqual(['Grace Hopper'])
    expect(chipsIn('Cc:')).toEqual(['Ada'])
  })
})
