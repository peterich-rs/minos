import { Component, type ErrorInfo, type ReactNode } from "react";

type Props = {
  children: ReactNode;
  /** Optional label for which subtree failed. */
  label?: string;
};

type State = {
  error: Error | null;
  info: string | null;
};

/**
 * Surfaces render crashes instead of a blank cream body background.
 * React has no root error boundary by default — one bad Sessions row
 * previously unmounted the whole app.
 */
export class ErrorBoundary extends Component<Props, State> {
  state: State = { error: null, info: null };

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { error };
  }

  componentDidCatch(error: Error, info: ErrorInfo): void {
    // Keep stack in state for on-screen debug (no remote logging yet).
    this.setState({
      error,
      info: info.componentStack ?? null,
    });
    console.error(
      `[ErrorBoundary${this.props.label ? `:${this.props.label}` : ""}]`,
      error,
      info.componentStack,
    );
  }

  render() {
    const { error, info } = this.state;
    if (!error) return this.props.children;

    return (
      <div className="flex h-full min-h-0 w-full flex-col gap-3 overflow-auto bg-rose-50 p-6 text-rose-950">
        <div className="text-[15px] font-semibold">
          UI crashed{this.props.label ? ` (${this.props.label})` : ""}
        </div>
        <pre className="whitespace-pre-wrap break-words rounded-lg border border-rose-200 bg-white p-3 font-mono text-[12px] leading-relaxed text-rose-900">
          {error.name}: {error.message}
        </pre>
        {info ? (
          <pre className="whitespace-pre-wrap break-words rounded-lg border border-rose-100 bg-white/80 p-3 font-mono text-[11px] leading-relaxed text-rose-800/90">
            {info}
          </pre>
        ) : null}
        <button
          type="button"
          className="self-start rounded-lg bg-ink px-3 py-1.5 text-[12px] font-semibold text-white"
          onClick={() => this.setState({ error: null, info: null })}
        >
          Try render again
        </button>
      </div>
    );
  }
}
