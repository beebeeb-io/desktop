import { Component, type ReactNode } from 'react'

/** Keep a render/update failure in setup from unmounting the entire WebView. */
export default class OnboardingErrorBoundary extends Component<
  { children: ReactNode },
  { failed: boolean }
> {
  state = { failed: false }

  static getDerivedStateFromError() {
    return { failed: true }
  }

  render() {
    if (!this.state.failed) return this.props.children

    // Do not include the exception: it may contain recovery input. Retrying
    // remounts setup, which reads the current session/unlock state from the host.
    return (
      <main style={{ padding: 32, maxWidth: 560, margin: '0 auto' }}>
        <section className="panel" role="alert" aria-labelledby="setup-error-title">
          <h1 id="setup-error-title">Setup couldn’t continue</h1>
          <p>An unexpected error interrupted setup. Try again to continue. You may need to re-enter your recovery phrase.</p>
          <button className="button amber" onClick={() => this.setState({ failed: false })}>
            Try setup again
          </button>
        </section>
      </main>
    )
  }
}
