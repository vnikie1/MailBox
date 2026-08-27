import { useCallback, useEffect, useState } from 'react'
import {
  AlignCenter,
  AlignJustify,
  AlignLeft,
  AlignRight,
  Baseline,
  Bold,
  Italic,
  Link as LinkIcon,
  List,
  ListOrdered,
  Minus,
  Quote,
  Type,
  Underline,
} from 'lucide-react'
import { $isLinkNode, TOGGLE_LINK_COMMAND } from '@lexical/link'
import {
  INSERT_ORDERED_LIST_COMMAND,
  INSERT_UNORDERED_LIST_COMMAND,
  REMOVE_LIST_COMMAND,
  $isListNode,
} from '@lexical/list'
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { $createQuoteNode, $isQuoteNode } from '@lexical/rich-text'
import { INSERT_HORIZONTAL_RULE_COMMAND } from '@lexical/extension'
import { $patchStyleText, $setBlocksType } from '@lexical/selection'
import { $findMatchingParent, mergeRegister } from '@lexical/utils'
import {
  $createParagraphNode,
  $getSelection,
  $isRangeSelection,
  $isElementNode,
  FORMAT_ELEMENT_COMMAND,
  FORMAT_TEXT_COMMAND,
  SELECTION_CHANGE_COMMAND,
  COMMAND_PRIORITY_LOW,
  type LexicalNode,
} from 'lexical'

import { IconButton, Menu, MenuItem, Tooltip, TooltipGroup } from '@/ui'

import styles from './FormatBar.module.css'

/**
 * The compose format bar. docs/01 §6 draws it, docs/06 Phase 7 names its contents.
 *
 * **Exactly Mail's set and nothing more**, and the restraint is the design rather than a
 * shortcut. Every control here produces a node the *recipient's* client has to render, and mail
 * clients are the least capable renderers in software — a table, a float, a background image
 * looks right in the composer and arrives broken somewhere else, and the sender never learns
 * that it did. A format bar is a promise about what will survive the journey.
 *
 * The buttons reflect the caret. A bold button that does not light up inside bold text leaves
 * the user pressing it to find out, which turns formatting into guesswork.
 */

/**
 * The colours the bar offers.
 *
 * A fixed list rather than a colour picker, and that is the point. A picker invites a pale
 * yellow that is unreadable on the recipient's white background, or on their dark one — the
 * sender sees it against the composer's background and never against theirs. These eight are
 * legible either way.
 *
 * Written as hex on purpose: this is a value travelling in an outgoing message, not a colour
 * in this app's interface, so the token layer standing rule 1 protects does not apply. It has
 * to be an absolute colour, because the recipient's client has no idea what our tokens mean.
 */
const COLOURS: { name: string; value: string }[] = [
  { name: 'Automatic', value: '' },
  { name: 'Red', value: '#c0392b' },
  { name: 'Orange', value: '#c8631b' },
  { name: 'Green', value: '#1e7a3c' },
  { name: 'Blue', value: '#1f5fbf' },
  { name: 'Purple', value: '#6b3fa0' },
  { name: 'Grey', value: '#5f6b76' },
  { name: 'Black', value: '#000000' },
]

/**
 * Sizes as absolute points rather than relative units.
 *
 * `em` and `%` compound: nested quoting multiplies them, and a reply to a reply to a reply
 * arrives at four points or forty. Mail sends points and so does everything that renders
 * predictably in Outlook.
 */
const SIZES: { name: string; value: string }[] = [
  { name: 'Small', value: '10pt' },
  { name: 'Medium', value: '12pt' },
  { name: 'Large', value: '14pt' },
  { name: 'Huge', value: '18pt' },
]

type Alignment = 'left' | 'center' | 'right' | 'justify'

interface ActiveState {
  bold: boolean
  italic: boolean
  underline: boolean
  bullet: boolean
  number: boolean
  quote: boolean
  link: boolean
  alignment: Alignment | null
}

const NOTHING: ActiveState = {
  bold: false,
  italic: false,
  underline: false,
  bullet: false,
  number: false,
  quote: false,
  link: false,
  alignment: null,
}

/** The alignment of the block the caret is in, or `null` for the default. */
function readAlignment(node: LexicalNode): Alignment | null {
  const block = $findMatchingParent(
    node,
    (candidate) => $isElementNode(candidate) && !candidate.isInline(),
  )

  if (block === null || !$isElementNode(block)) return null

  const format = block.getFormatType()

  // `start` and `end` are logical directions rather than the four the bar offers, and the empty
  // string is "no alignment set" — which is not the same as left, and must not light the Left
  // button up for every ordinary paragraph.
  return format === '' || format === 'start' || format === 'end' ? null : format
}

export function FormatBar() {
  const [editor] = useLexicalComposerContext()
  const [active, setActive] = useState<ActiveState>(NOTHING)

  /** Reads what the caret is currently inside, so the buttons can show it. */
  const sync = useCallback(() => {
    const selection = $getSelection()
    if (!$isRangeSelection(selection)) {
      setActive(NOTHING)
      return
    }

    const anchor = selection.anchor.getNode()
    const list = $findMatchingParent(anchor, $isListNode)

    setActive({
      bold: selection.hasFormat('bold'),
      italic: selection.hasFormat('italic'),
      underline: selection.hasFormat('underline'),
      bullet: list !== null && $isListNode(list) && list.getListType() === 'bullet',
      number: list !== null && $isListNode(list) && list.getListType() === 'number',
      quote: $findMatchingParent(anchor, $isQuoteNode) !== null,
      link: $findMatchingParent(anchor, $isLinkNode) !== null,
      // Read from the block the caret is in, not from the selection. Alignment is a property of
      // the paragraph, and asking the selection for it reports nothing whenever the caret is
      // inside a link or a list item rather than directly in the block.
      alignment: readAlignment(anchor),
    })
  }, [])

  useEffect(
    () =>
      mergeRegister(
        editor.registerUpdateListener(({ editorState }) => {
          editorState.read(sync)
        }),
        editor.registerCommand(
          SELECTION_CHANGE_COMMAND,
          () => {
            sync()
            return false
          },
          COMMAND_PRIORITY_LOW,
        ),
      ),
    [editor, sync],
  )

  const toggleList = useCallback(
    (type: 'bullet' | 'number', currentlyOn: boolean) => {
      if (currentlyOn) {
        editor.dispatchCommand(REMOVE_LIST_COMMAND, undefined)
        return
      }
      editor.dispatchCommand(
        type === 'bullet' ? INSERT_UNORDERED_LIST_COMMAND : INSERT_ORDERED_LIST_COMMAND,
        undefined,
      )
    },
    [editor],
  )

  /**
   * Sets the block alignment, or clears it when it is already set.
   *
   * A toggle rather than a one-way set: the alternative leaves no way back to the default
   * except undo, and "left" is not the same as "unset" — a left-aligned block carries an
   * explicit `text-align` that overrides the recipient's own direction, which is wrong for
   * anyone reading right-to-left.
   */
  const align = useCallback(
    (to: Alignment) => {
      editor.dispatchCommand(FORMAT_ELEMENT_COMMAND, active.alignment === to ? '' : to)
    },
    [editor, active.alignment],
  )

  const toggleQuote = useCallback(() => {
    editor.update(() => {
      const selection = $getSelection()
      if (!$isRangeSelection(selection)) return

      // Back to a paragraph when it is already a quote, so the button is a toggle rather than
      // a one-way trip the user has to undo their way out of.
      $setBlocksType(selection, () => (active.quote ? $createParagraphNode() : $createQuoteNode()))
    })
  }, [editor, active.quote])

  /**
   * Applies an inline style to the selection.
   *
   * An empty value clears the property rather than writing `color: ` into the markup, which
   * some clients treat as a parse error for the whole declaration and then drop every other
   * style on the element with it.
   */
  const patch = useCallback(
    (property: string, value: string) => {
      editor.update(() => {
        const selection = $getSelection()
        if (!$isRangeSelection(selection)) return
        $patchStyleText(selection, { [property]: value === '' ? null : value })
      })
    },
    [editor],
  )

  const toggleLink = useCallback(() => {
    if (active.link) {
      editor.dispatchCommand(TOGGLE_LINK_COMMAND, null)
      return
    }

    const url = window.prompt('Link address')
    if (url === null) return

    const trimmed = url.trim()
    if (trimmed === '') return

    // Only the two schemes a message may carry. `javascript:` in a link the user is about to
    // send under their own name is the same hazard as one in a message body, and it would be
    // signed by them rather than by a stranger.
    if (!/^https?:\/\//i.test(trimmed) && !/^mailto:/i.test(trimmed)) {
      editor.dispatchCommand(TOGGLE_LINK_COMMAND, `https://${trimmed}`)
      return
    }

    editor.dispatchCommand(TOGGLE_LINK_COMMAND, trimmed)
  }, [editor, active.link])

  return (
    <div className={styles.bar} role="toolbar" aria-label="Formatting">
      <TooltipGroup>
        <Tooltip
          content="Bold"
          trigger={
            <IconButton
              icon={Bold}
              label="Bold"
              size="sm"
              aria-pressed={active.bold}
              onClick={() => {
                editor.dispatchCommand(FORMAT_TEXT_COMMAND, 'bold')
              }}
            />
          }
        />
        <Tooltip
          content="Italic"
          trigger={
            <IconButton
              icon={Italic}
              label="Italic"
              size="sm"
              aria-pressed={active.italic}
              onClick={() => {
                editor.dispatchCommand(FORMAT_TEXT_COMMAND, 'italic')
              }}
            />
          }
        />
        <Tooltip
          content="Underline"
          trigger={
            <IconButton
              icon={Underline}
              label="Underline"
              size="sm"
              aria-pressed={active.underline}
              onClick={() => {
                editor.dispatchCommand(FORMAT_TEXT_COMMAND, 'underline')
              }}
            />
          }
        />
      </TooltipGroup>

      <span className={styles.divider} aria-hidden />

      <Menu
        label="Text Colour"
        trigger={<IconButton icon={Baseline} label="Text Colour" size="sm" />}
      >
        {COLOURS.map((colour) => (
          <MenuItem
            key={colour.name}
            label={colour.name}
            onClick={() => {
              patch('color', colour.value)
            }}
          />
        ))}
      </Menu>

      <Menu label="Text Size" trigger={<IconButton icon={Type} label="Text Size" size="sm" />}>
        {SIZES.map((size) => (
          <MenuItem
            key={size.name}
            label={size.name}
            onClick={() => {
              patch('font-size', size.value)
            }}
          />
        ))}
      </Menu>

      <span className={styles.divider} aria-hidden />

      <TooltipGroup>
        <Tooltip
          content="Align Left"
          trigger={
            <IconButton
              icon={AlignLeft}
              label="Align Left"
              size="sm"
              aria-pressed={active.alignment === 'left'}
              onClick={() => {
                align('left')
              }}
            />
          }
        />
        <Tooltip
          content="Align Centre"
          trigger={
            <IconButton
              icon={AlignCenter}
              label="Align Centre"
              size="sm"
              aria-pressed={active.alignment === 'center'}
              onClick={() => {
                align('center')
              }}
            />
          }
        />
        <Tooltip
          content="Align Right"
          trigger={
            <IconButton
              icon={AlignRight}
              label="Align Right"
              size="sm"
              aria-pressed={active.alignment === 'right'}
              onClick={() => {
                align('right')
              }}
            />
          }
        />
        <Tooltip
          content="Justify"
          trigger={
            <IconButton
              icon={AlignJustify}
              label="Justify"
              size="sm"
              aria-pressed={active.alignment === 'justify'}
              onClick={() => {
                align('justify')
              }}
            />
          }
        />
      </TooltipGroup>

      <span className={styles.divider} aria-hidden />

      <TooltipGroup>
        <Tooltip
          content="Bulleted List"
          trigger={
            <IconButton
              icon={List}
              label="Bulleted List"
              size="sm"
              aria-pressed={active.bullet}
              onClick={() => {
                toggleList('bullet', active.bullet)
              }}
            />
          }
        />
        <Tooltip
          content="Numbered List"
          trigger={
            <IconButton
              icon={ListOrdered}
              label="Numbered List"
              size="sm"
              aria-pressed={active.number}
              onClick={() => {
                toggleList('number', active.number)
              }}
            />
          }
        />
        <Tooltip
          content="Quote"
          trigger={
            <IconButton
              icon={Quote}
              label="Quote"
              size="sm"
              aria-pressed={active.quote}
              onClick={toggleQuote}
            />
          }
        />
        <Tooltip
          content={active.link ? 'Remove Link' : 'Add Link'}
          trigger={
            <IconButton
              icon={LinkIcon}
              label={active.link ? 'Remove Link' : 'Add Link'}
              size="sm"
              aria-pressed={active.link}
              onClick={toggleLink}
            />
          }
        />
        <Tooltip
          content="Separator"
          trigger={
            <IconButton
              icon={Minus}
              label="Separator"
              size="sm"
              onClick={() => {
                editor.dispatchCommand(INSERT_HORIZONTAL_RULE_COMMAND, undefined)
              }}
            />
          }
        />
      </TooltipGroup>
    </div>
  )
}
