import { Component } from 'react';
import type { ErrorInfo, ReactNode } from 'react';
import './ComponentErrorBoundary.css';

interface Props {
  name: string;
  children: ReactNode;
  fallback?: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
}

export default class ComponentErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null };

  static getDerivedStateFromError(error: Error): State {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[${this.props.name}] ErrorBoundary caught:`, error, info.componentStack);
  }

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) return this.props.fallback;
      return (
        <div className="component-error-boundary">
          <span className="ceb-icon" aria-hidden="true">{'\u26A0'}</span>
          <span className="ceb-name">{this.props.name}</span>
          <span className="ceb-msg">{this.state.error?.message ?? 'Unknown error'}</span>
          <button
            className="ceb-retry"
            onClick={() => this.setState({ hasError: false, error: null })}
          >
            Reload
          </button>
        </div>
      );
    }
    return this.props.children;
  }
}
