import { useCallback, useEffect, useRef } from 'react'
import { $generateHtmlFromNodes, $generateNodesFromDOM } from '@lexical/html'
import { LinkNode } from '@lexical/link'
import { ListItemNode, ListNode } from '@lexical/list'
// From `@lexical/extension` rather than `@lexical/react`: the react package's copy is
// deprecated in favour of this one, which is the pure-Lexical implementation.
import { HorizontalRuleNode } from '@lexical/extension'
import { LexicalComposer } from '@lexical/react/LexicalComposer'
import { ContentEditable } from '@lexical/react/LexicalContentEditable'
import { LexicalErrorBoundary } from '@lexical/react/LexicalErrorBoundary'
import { HistoryPlugin } from '@lexical/react/LexicalHistoryPlugin'
import { OnChangePlugin } from '@lexical/react/LexicalOnChangePlugin'
import { RichTextPlugin } from '@lexical/react/LexicalRichTextPlugin'
import { useLexicalComposerContext } from '@lexical/react/LexicalComposerContext'
import { HeadingNode, QuoteNode } from '@lexical/rich-text'
import { $getRoot, $insertNodes, type EditorState, type LexicalEditor } from 'lexical'

import { FormatBar } from './FormatBar'

import styles from './Editor.module.css'

/**
 * The compose body. docs/01 §6, docs/06 Phase 7.
 *
 * Lexical, because a `contenteditable` behaves differently in every browser engine and the
 * ways it differs are exactly the ways that matter here — where the caret lands after a list
 * item, what pasting from Word produces, whether undo groups a word or a character. Lexical
 * owns a model and renders from it, so those are decisions rather than accidents.
 *
 * **The editor holds HTML the user is about to send under their own name.** That makes pasted
 * content the same class of input as a message body: `$generateNodesFromDOM` builds nodes from
 * a parsed document rather than injecting markup, and the node allow-list below is what the
 * document can contain. Anything outside it — a script, an iframe, an event handler — has no
 * node to become and is dropped in the conversion rather than filtered afterwards.
 */

/**
 * Exactly Mail's format bar and nothing more. docs/06 Phase 7 names the set: bold, italic,
 * underline, colour, size, alignment, lists, quote, link, horizontal rule.
 *
 * A wider set is not generosity. Every node type here is one the recipient's client has to
 * render, and mail clients are the least capable renderers in software — a table or a nested
 * float that looks right here arrives broken somewhere else, and the sender never learns.
 */
const NODES = [HeadingNode, QuoteNode, ListNode, ListItemNode, LinkNode, HorizontalRuleNode]

export interface EditorProps {
  /** Initial HTML, used once. Later changes to this prop are ignored. */
  initialHtml?: string
  onChange: (html: string, text: string) => void
  ariaLabel: string
}

/**
 * Loads the initial HTML exactly once.
 *
 * Not in a `useEffect` on `initialHtml`, and not on every render: re-applying it would discard
 * whatever the user had typed since. A reply's quoted original arrives once, at the start, and
 * from then on the editor's own state is the truth.
 */
function InitialContent({ html }: { html: string }) {
  const [editor] = useLexicalComposerContext()
  const applied = useRef(false)

  useEffect(() => {
    if (applied.current || html.trim() === '') return
    applied.current = true

    editor.update(() => {
      // Parsed as a document and converted to nodes, rather than set as innerHTML. Anything
      // without a node type in NODES simply has nothing to become.
      const parsed = new DOMParser().parseFromString(html, 'text/html')
      const nodes = $generateNodesFromDOM(editor, parsed)

      const root = $getRoot()
      root.clear()
      root.select()
      $insertNodes(nodes)

      // The caret belongs above the quote, where the reply gets typed — not at the end of
      // the message being replied to.
      root.selectStart()
    })
  }, [editor, html])

  return null
}

export function Editor({ initialHtml = '', onChange, ariaLabel }: EditorProps) {
  const handleChange = useCallback(
    (state: EditorState, editor: LexicalEditor) => {
      state.read(() => {
        // Both forms are produced here, from the same state, at the same moment. Deriving the
        // plain text from the HTML afterwards is guesswork; the editor knows what was typed.
        onChange($generateHtmlFromNodes(editor, null), $getRoot().getTextContent())
      })
    },
    [onChange],
  )

  return (
    <LexicalComposer
      initialConfig={{
        namespace: 'halcyon-compose',
        nodes: NODES,
        theme: {
          paragraph: styles.paragraph ?? '',
          quote: styles.quote ?? '',
          // `?? ''` throughout because CSS Modules types every class as possibly undefined and
          // Lexical's theme wants definite strings. An empty class is the honest fallback: the
          // node still renders, unstyled, rather than the editor refusing to build.
          list: {
            ul: styles.bulletList ?? '',
            ol: styles.numberList ?? '',
            listitem: styles.listItem ?? '',
          },
          text: {
            bold: styles.bold ?? '',
            italic: styles.italic ?? '',
            underline: styles.underline ?? '',
            strikethrough: styles.strikethrough ?? '',
          },
          link: styles.link ?? '',
        },
        // Standing rule 13 — degrade visibly, never take the window down. A message half
        // written is worth more than a clean stack trace.
        onError: (error: Error) => {
          console.error('[compose editor]', error)
        },
      }}
    >
      <div className={styles.shell}>
        {/* Inside the composer, because it needs the editor context. Above the text, as
            docs/01 §6 draws it. */}
        <FormatBar />

        <RichTextPlugin
          contentEditable={<ContentEditable className={styles.content} aria-label={ariaLabel} />}
          ErrorBoundary={LexicalErrorBoundary}
        />
        <HistoryPlugin />
        <OnChangePlugin onChange={handleChange} ignoreSelectionChange />
        <InitialContent html={initialHtml} />
      </div>
    </LexicalComposer>
  )
}
