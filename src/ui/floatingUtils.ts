import { cloneElement, type Attributes, type ReactElement, type Ref } from 'react'
import type { Placement } from '@floating-ui/react'

/**
 * A floating layer's trigger: any single element that can take a ref and the interaction
 * props Floating UI generates. In React 19 `ref` is an ordinary prop on function
 * components, so every primitive in `src/ui` that spreads its rest props qualifies.
 */
export interface TriggerProps {
  ref?: Ref<HTMLElement>
}

export type TriggerElement = ReactElement<TriggerProps>

/**
 * Clone a trigger with the props Floating UI produced for it.
 *
 * The trigger's **own** props are handed to `getProps` rather than being left to survive
 * the clone. Floating UI merges what it is given with what it generates, calling both
 * handlers; `cloneElement` does not merge, it replaces. Passing them separately meant a
 * trigger's `onClick` was silently dropped the moment the primitive generated one of its
 * own — which `useDismiss({ referencePress: true })` does. A tooltip-wrapped button
 * therefore looked completely normal and did nothing when clicked.
 *
 * `extra` is for props that belong to the clone rather than to the merge — the ref, in
 * practice, which Floating UI does not merge.
 *
 * The cast is the one unavoidable piece of unsoundness in this file, and it is why the
 * function exists at all rather than being inlined four times: `getReferenceProps()`
 * returns an open record of event handlers and ARIA attributes, which no element's prop
 * type will accept without one. Confining it here means the four floating primitives
 * stay fully typed.
 */
export function withTriggerProps(
  trigger: TriggerElement,
  getProps: (userProps?: Record<string, unknown>) => Record<string, unknown>,
  extra: Record<string, unknown> = {},
): ReactElement {
  const own = trigger.props as Record<string, unknown>
  const merged = getProps(own)

  // `extra` is applied after the merge, not through it. Floating UI's prop merger is for
  // event handlers and ARIA; a ref passed through it does not reliably survive, and a
  // trigger with no ref is a trigger the focus manager cannot return focus to.
  return cloneElement(trigger, { ...merged, ...extra } as Partial<TriggerProps> & Attributes)
}

/**
 * Where a floating layer should grow from.
 *
 * A popover that scales up from its own centre reads as arriving from nowhere; one that
 * scales from the edge nearest its trigger reads as coming out of the control you just
 * clicked, which is what macOS does. `placement` is the resolved placement after flip(),
 * so this stays right when a layer has been flipped to the other side of its trigger.
 */
export function transformOriginFor(placement: Placement): string {
  const [side, alignment] = placement.split('-')

  const block =
    side === 'top' ? 'bottom' : side === 'bottom' ? 'top' : alignment === 'end' ? 'bottom' : 'top'
  const inline =
    side === 'left' ? 'right' : side === 'right' ? 'left' : alignment === 'end' ? 'right' : 'left'

  return `${block} ${inline}`
}
