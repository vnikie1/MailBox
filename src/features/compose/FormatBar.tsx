import { useCallback, useEffect, useState } from 'react'
import { Bold, Italic, Link as LinkIcon, List, ListOrdered, Quote, Underline } from 'lucide-react'
import { $isLinkNode, TOGGLE_LINK_COMMAND } from '@lexical/link'
import {
  INSERT_ORDERED_LIST_COMMAND,
  INSERT_UNORDERED_LIST_COMMAND,
  REMOVE_LIST_COMMAND,
  $isListNode,
} from '@lexical/list'
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { $createQuoteNode, $isQuoteNode } from '@lexical/rich-text'
import { $setBlocksType } from '@lexical/selection'
import { $findMatchingParent, mergeRegister } from '@lexical/utils'
import {
  $createParagraphNode,
  $getSelection,
  $isRangeSelection,
  FORMAT_TEXT_COMMAND,
  SELECTION_CHANGE_COMMAND,
  COMMAND_PRIORITY_LOW,
} from 'lexical'

import { IconButton, Tooltip, TooltipGroup } from '@/ui'

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

interface ActiveState {
  bold: boolean
  italic: boolean
  underline: boolean
  bullet: boolean
  number: boolean
  quote: boolean
  link: boolean
}

const NOTHING: ActiveState = {
  bold: false,
  italic: false,
  underline: false,
  bullet: false,
  number: false,
  quote: false,
  link: false,
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

  const toggleQuote = useCallback(() => {
    editor.update(() => {
      const selection = $getSelection()
      if (!$isRangeSelection(selection)) return

      // Back to a paragraph when it is already a quote, so the button is a toggle rather than
      // a one-way trip the user has to undo their way out of.
      $setBlocksType(selection, () => (active.quote ? $createParagraphNode() : $createQuoteNode()))
    })
  }, [editor, active.quote])

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
      </TooltipGroup>
    </div>
  )
}
