import { Component } from 'react'
import type { ErrorInfo, ReactNode } from 'react'

interface Props {
  /** Shown in the console so a hidden widget is still diagnosable. */
  name: string
  children: ReactNode
}

/**
 * Error boundary for optional header widgets: a widget that throws (an API returning an
 * unexpected shape, say) renders nothing instead of taking the whole shell down. Remount it
 * with a `key` to retry, e.g. when the time range changes.
 */
export class WidgetBoundary extends Component<Props, { failed: boolean }> {
  state = { failed: false }

  static getDerivedStateFromError(): { failed: boolean } {
    return { failed: true }
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    console.error(`Header widget "${this.props.name}" failed and is hidden`, error, info.componentStack)
  }

  render(): ReactNode {
    return this.state.failed ? null : this.props.children
  }
}
