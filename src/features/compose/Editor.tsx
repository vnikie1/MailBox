import { useCallback, useEffect, useMemo, useRef, type RefObject } from 'react'
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
import {
  $createParagraphNode,
  $getNodeByKey,
  $getRoot,
  $isDecoratorNode,
  $isElementNode,
  $isTextNode,
  TextNode,
  getStaticNodeConfig,
  type DOMConversionMap,
  type EditorState,
  type LexicalEditor,
  type LexicalNode,
  type NodeKey,
} from 'lexical'

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

/**
 * Who changed the text: the user, or the editor putting in what it was given — the content it
 * opened with, or a signature swapped for another account's.
 *
 * The difference is the difference between a message someone has written and one they have only
 * looked at. Without it the compose window could not tell a reply opened and closed again from
 * one with a paragraph typed into it, and asked to save both (L25, 2026-11-14 audit); and the
 * Signatures pane saved every signature back the moment it was shown (M47).
 */
export type ChangeOrigin = 'user' | 'load'

/** The signature, and where it goes when there is none in the message to replace. */
export interface EditorSignature {
  html: string
  /** `above` the quoted original, or `below` everything. */
  placement: 'above' | 'below'
}

/** What a message opens with, either side of where the signature goes. */
export interface EditorContent {
  /** The line to type on, a `mailto:` body, or a quote the signature goes below. */
  before: string
  /** A quote the signature goes above. */
  after: string
}

export interface EditorProps {
  /** Initial HTML, used once. Later changes to this prop are ignored. */
  initialHtml?: string
  /**
   * The same, in two parts with the signature between them. Applied once, when it stops being
   * `null`, with whatever `signature` is at that moment.
   */
  initialContent?: EditorContent | null
  /**
   * The signature. Once the content has loaded, a change to it replaces the signature in the
   * message where it stands — choosing another From, Mail's way (M46). A signature the user has
   * deleted stays deleted.
   */
  signature?: EditorSignature | null
  onChange: (html: string, text: string, origin: ChangeOrigin) => void
  ariaLabel: string
}

/** Marks the updates the editor makes by itself. See `ChangeOrigin`. */
const LOADED = 'halcyon-loaded'

/**
 * The styles the format bar writes, kept when markup is loaded.
 *
 * Lexical's own importers keep bold, italic, underline and the like, which they turn into its
 * text formats, and drop every other style on a `<span>` — so a signature written in red at 18px
 * came back black and body-sized each time it was loaded, and was saved that way (M47). These two
 * are what `FormatBar` can set and so what this editor can have written.
 */
const KEPT_STYLES = ['color', 'font-size'] as const

/**
 * Lexical's text importers, each also carrying `KEPT_STYLES` across to the text inside it while
 * `keeping` is set.
 *
 * Only while loading, not for every paste: pasted markup is someone else's styling, and keeping
 * its colour is how white text from a dark web page ends up invisible in a message and in the
 * recipient's inbox. What this editor loads is what this editor wrote.
 */
function styleKeepingImporters(keeping: RefObject<boolean>): DOMConversionMap {
  const importers: DOMConversionMap = {}

  // Lexical 0.49 declares a node's importers in its `$config()` and fills in the static
  // `importDOM` from it lazily, when an editor first registers the node. This one runs before any
  // editor exists, so it asks for the config itself.
  getStaticNodeConfig(TextNode)

  for (const [tag, importer] of Object.entries(TextNode.importDOM?.() ?? {})) {
    // `#text` is a text node, which has no style to keep.
    if (tag.startsWith('#')) continue

    importers[tag] = (element: HTMLElement) => {
      const conversion = importer(element)
      if (conversion === null) return null

      return {
        ...conversion,
        conversion: (node: HTMLElement) => {
          const output = conversion.conversion(node)
          if (!keeping.current || output === null) return output

          const style = KEPT_STYLES.flatMap((property) => {
            const value = node.style.getPropertyValue(property)
            return value === '' ? [] : [`${property}: ${value};`]
          }).join(' ')
          if (style === '') return output

          const { forChild } = output
          return {
            ...output,
            forChild: (child: LexicalNode, parent: LexicalNode | null | undefined) => {
              const converted = forChild === undefined ? child : forChild(child, parent)
              if ($isTextNode(converted)) {
                converted.setStyle(`${converted.getStyle()} ${style}`.trim())
              }
              return converted
            },
          }
        },
      }
    }
  }

  return importers
}

/**
 * Markup as top-level blocks: inline runs wrapped in paragraphs, as the root requires.
 *
 * Built by hand rather than through `$insertNodes`, because the signature's blocks have to be
 * known by key afterwards to be replaced, and an insertion that merges into the paragraph the
 * selection happens to be in would lose them.
 */
function $blocksFrom(editor: LexicalEditor, html: string): LexicalNode[] {
  if (html.trim() === '') return []

  // Parsed as a document and converted to nodes, rather than set as innerHTML. Anything without
  // a node type in NODES simply has nothing to become.
  const parsed = new DOMParser().parseFromString(html, 'text/html')
  const blocks: LexicalNode[] = []
  let paragraph: ReturnType<typeof $createParagraphNode> | null = null

  for (const node of $generateNodesFromDOM(editor, parsed)) {
    if (($isElementNode(node) || $isDecoratorNode(node)) && !node.isInline()) {
      paragraph = null
      blocks.push(node)
      continue
    }

    if (paragraph === null) {
      paragraph = $createParagraphNode()
      blocks.push(paragraph)
    }
    paragraph.append(node)
  }

  return blocks
}

/** Runs a conversion with the format bar's styles kept. See `styleKeepingImporters`. */
function keepingStyles<T>(keeping: RefObject<boolean>, convert: () => T): T {
  keeping.current = true
  try {
    return convert()
  } finally {
    keeping.current = false
  }
}

/** The signature's markup, as it has always gone into a message. */
function signatureMarkup(signature: EditorSignature | null | undefined): string {
  return signature === null || signature === undefined || signature.html.trim() === ''
    ? ''
    : `<div>${signature.html}</div>`
}

/**
 * Loads the initial content exactly once, and keeps the signature in it current.
 *
 * Not in a `useEffect` that re-applies the content on every change: that would discard whatever
 * the user had typed since. A reply's quoted original arrives once, at the start, and from then
 * on the editor's own state is the truth — except for the signature, which is replaced in place,
 * by the keys of the blocks it was loaded as.
 */
function InitialContent({
  html,
  content,
  signature,
  keepStyles,
}: {
  html: string
  content: EditorContent | null
  signature: EditorSignature | null
  keepStyles: RefObject<boolean>
}) {
  const [editor] = useLexicalComposerContext()
  const applied = useRef(false)
  /** The blocks the signature is in, once loaded. Empty if there was none to load. */
  const signatureKeys = useRef<NodeKey[]>([])
  /** Whether a signature went in at all, so one the user deleted is not put back. */
  const signed = useRef(false)
  /** The first block of `after`, which a signature placed above the quote goes before. */
  const afterKey = useRef<NodeKey | null>(null)
  const loadedSignature = useRef('')

  useEffect(() => {
    if (applied.current) return

    const opening: EditorContent | null =
      content ?? (html.trim() === '' ? null : { before: html, after: '' })
    if (opening === null) return
    applied.current = true

    const markup = signatureMarkup(signature)
    loadedSignature.current = markup

    editor.update(
      () => {
        const [before, signatureBlocks, after] = keepingStyles(keepStyles, () => [
          $blocksFrom(editor, opening.before),
          $blocksFrom(editor, markup),
          $blocksFrom(editor, opening.after),
        ])

        signatureKeys.current = signatureBlocks.map((node) => node.getKey())
        signed.current = signatureBlocks.length > 0
        afterKey.current = after[0]?.getKey() ?? null

        // Nothing to load leaves the editor's own empty paragraph to type in.
        const blocks = [...before, ...signatureBlocks, ...after]
        if (blocks.length === 0) return

        const root = $getRoot()
        root.clear()
        root.append(...blocks)

        // The caret belongs above the quote, where the reply gets typed — not at the end of
        // the message being replied to.
        root.selectStart()
      },
      { tag: LOADED },
    )
  }, [editor, html, content, signature, keepStyles])

  useEffect(() => {
    if (!applied.current) return

    const markup = signatureMarkup(signature)
    if (markup === loadedSignature.current) return
    loadedSignature.current = markup

    editor.update(
      () => {
        const present = signatureKeys.current
          .map((key) => $getNodeByKey(key))
          .filter((node): node is LexicalNode => node?.isAttached() === true)

        // Deleted by the user, all of it. Theirs to delete; changing From does not undo it.
        if (signed.current && present.length === 0) return

        const replacement = keepingStyles(keepStyles, () => $blocksFrom(editor, markup))
        signatureKeys.current = replacement.map((node) => node.getKey())
        signed.current = replacement.length > 0

        const first = present[0]
        if (first !== undefined) {
          for (const node of replacement) first.insertBefore(node)
          for (const node of present) node.remove()
          return
        }

        // None to replace: put it where it would have gone, below a blank line to type on.
        if (replacement.length === 0) return
        const spaced = [$createParagraphNode(), ...replacement]
        const anchor = afterKey.current === null ? null : $getNodeByKey(afterKey.current)

        if (signature?.placement === 'above' && anchor?.isAttached() === true) {
          for (const node of spaced) anchor.insertBefore(node)
        } else {
          $getRoot().append(...spaced)
        }
      },
      { tag: LOADED },
    )
  }, [editor, signature, keepStyles])

  return null
}

export function Editor({
  initialHtml = '',
  initialContent = null,
  signature = null,
  onChange,
  ariaLabel,
}: EditorProps) {
  const handleChange = useCallback(
    (state: EditorState, editor: LexicalEditor, tags: Set<string>) => {
      state.read(() => {
        // Both forms are produced here, from the same state, at the same moment. Deriving the
        // plain text from the HTML afterwards is guesswork; the editor knows what was typed.
        onChange(
          $generateHtmlFromNodes(editor, null),
          $getRoot().getTextContent(),
          tags.has(LOADED) ? 'load' : 'user',
        )
      })
    },
    [onChange],
  )

  // Read by the importers below, which are fixed when the editor is created: the composer takes
  // its configuration once.
  const keepStyles = useRef(false)
  const importers = useMemo(() => styleKeepingImporters(keepStyles), [])

  return (
    <LexicalComposer
      initialConfig={{
        namespace: 'halcyon-compose',
        nodes: NODES,
        html: { import: importers },
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
        <InitialContent
          html={initialHtml}
          content={initialContent}
          signature={signature}
          keepStyles={keepStyles}
        />
      </div>
    </LexicalComposer>
  )
}
