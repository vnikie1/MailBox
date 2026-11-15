import { Component, type ErrorInfo, type ReactNode } from 'react'

export interface ErrorBoundaryProps {
  children: ReactNode
  /** Shown in place of the children once they have thrown. */
  fallback: ReactNode
  /**
   * Clears a caught error when it changes, so the next thing shown gets a fresh attempt.
   *
   * A primitive rather than an object, because it is compared with `!==` on every render. For a
   * message pane, the message's id.
   */
  resetKey?: string | number | undefined
  /** Names the boundary in the console, so a caught error says where it was caught. */
  label: string
}

interface ErrorBoundaryState {
  failed: boolean
  /** The `resetKey` the current state belongs to. */
  key: string | number | undefined
}

/**
 * Contains a render error to the part of the window that threw.
 *
 * React's answer to an error thrown while rendering, with no boundary above it, is to unmount
 * the whole tree — so one message whose markup tripped something in the reader left the entire
 * window blank, sidebar and list included, until the app was restarted. That is M23 in the
 * 2026-11-14 audit, where a table of 120,000 rows did it.
 *
 * A class, because a boundary can only be one: React has no hook for this.
 */
export class ErrorBoundary extends Component<ErrorBoundaryProps, ErrorBoundaryState> {
  override state: ErrorBoundaryState = { failed: false, key: this.props.resetKey }

  static getDerivedStateFromError(): Partial<ErrorBoundaryState> {
    return { failed: true }
  }

  static getDerivedStateFromProps(
    props: ErrorBoundaryProps,
    state: ErrorBoundaryState,
  ): Partial<ErrorBoundaryState> | null {
    return props.resetKey === state.key ? null : { failed: false, key: props.resetKey }
  }

  override componentDidCatch(error: unknown, info: ErrorInfo): void {
    console.error(`[${this.props.label}]`, error, info.componentStack)
  }

  override render(): ReactNode {
    return this.state.failed ? this.props.fallback : this.props.children
  }
}
