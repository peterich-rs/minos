import { Component, type ErrorInfo, type ReactNode } from "react";
import { emitInitialRenderReady } from "@/shared/lib/initial-render-ready";

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
 *
 * Also re-emits `initial-render-ready` so a boot-time render crash still
 * reveals the Tauri window (error UI is a valid first surface).
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
    // Insurance for host reveal: App's useLayoutEffect may never run if
    // the tree throws before commit.
    emitInitialRenderReady();
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
      <div className="flex h-full min-h-0 w-full flex-col gap-3 overflow-auto bg-status-failed/10 p-6 text-status-failed">
        <div className="text-base font-semibold">
          UI crashed{this.props.label ? ` (${this.props.label})` : ""}
        </div>
        <pre className="whitespace-pre-wrap break-words rounded-lg border border-status-failed/30 bg-surface-raised p-3 font-mono text-xs leading-relaxed text-status-failed">
          {error.name}: {error.message}
        </pre>
        {info ? (
          <pre className="whitespace-pre-wrap break-words rounded-lg border border-status-failed/20 bg-surface-raised/80 p-3 font-mono text-2xs leading-relaxed text-status-failed/90">
            {info}
          </pre>
        ) : null}
        <button
          type="button"
          className="self-start rounded-lg bg-ink px-3 py-1.5 text-xs font-semibold text-surface"
          onClick={() => this.setState({ error: null, info: null })}
        >
          Try render again
        </button>
      </div>
    );
  }
}
