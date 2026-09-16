import { Component, type ReactNode } from "react";

export class ErrorBoundary extends Component<{ children: ReactNode }, { error: Error | null }> {
  state = { error: null as Error | null };
  static getDerivedStateFromError(error: Error) {
    return { error };
  }
  render() {
    if (this.state.error) {
      return (
        <div className="m-4 rounded border border-edge bg-panel p-4 text-sm text-ink">
          <div className="mb-2 font-semibold text-[#ff9500]">Globe failed to render</div>
          <div className="mb-3 text-xs text-dim">{String(this.state.error)}</div>
          <button
            className="rounded border border-edge px-2 py-1 text-xs hover:bg-edge"
            onClick={() => this.setState({ error: null })}
          >
            Retry
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
