import { Component, createRef } from 'react';
import type { ErrorInfo, ReactNode } from 'react';
import './ComponentErrorBoundary.css';

type Severity = 'crash' | 'timeout' | 'network';

interface Props {
  name: string;
  children: ReactNode;
  fallback?: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
  recovering: boolean;
  showStack: boolean;
  typed: boolean;
}

function classifyError(error: Error): Severity {
  const msg = error.message.toLowerCase();
  if (msg.includes('timeout') || msg.includes('timed out') || msg.includes('deadline')) return 'timeout';
  if (msg.includes('network') || msg.includes('fetch') || msg.includes('econnrefused')) return 'network';
  return 'crash';
}

/** Lightly colorize stack trace frames */
function renderStackTrace(stack: string | undefined): ReactNode {
  if (!stack) return null;
  const lines = stack.split('\n');
  return lines.map((line, i) => {
    const fnMatch = line.match(/^\s+at\s+([\w.<>]+)/);
    const fileMatch = line.match(/\((.*?):(\d+):\d+\)/);
    if (fnMatch && fileMatch) {
      return (
        <span key={i}>
          {'  at '}
          <span className="stack-fn">{fnMatch[1]}</span>
          {' ('}
          <span className="stack-file">{fileMatch[1]}</span>
          {':'}
          <span className="stack-line">{fileMatch[2]}</span>
          {')\n'}
        </span>
      );
    }
    return <span key={i}>{line}{'\n'}</span>;
  });
}

export default class ComponentErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null, recovering: false, showStack: false, typed: false };
  private retryRef = createRef<HTMLButtonElement>();
  private typeTimer: ReturnType<typeof setTimeout> | null = null;

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error(`[${this.props.name}] ErrorBoundary caught:`, error, info.componentStack);
  }

  componentDidUpdate(_pp: Props, prevState: State) {
    // start typewriter completion timer when error first appears
    if (this.state.hasError && !prevState.hasError) {
      this.typeTimer = setTimeout(() => this.setState({ typed: true }), 1800);
    }
  }

  componentWillUnmount() {
    if (this.typeTimer) clearTimeout(this.typeTimer);
  }

  private handleRetry = () => {
    const btn = this.retryRef.current;
    if (btn) {
      btn.classList.add('pulsing');
    }
    this.setState({ recovering: true });
    setTimeout(() => {
      this.setState({ hasError: false, error: null, recovering: false, showStack: false, typed: false });
    }, 450);
  };

  render() {
    if (this.state.hasError) {
      if (this.props.fallback) return this.props.fallback;

      const severity = this.state.error ? classifyError(this.state.error) : 'crash';
      const stack = this.state.error?.stack;

      return (
        <div
          className={`component-error-boundary${this.state.recovering ? ' recovering' : ''}`}
          data-severity={severity}
        >
          {/* animated warning triangle */}
          <svg className="ceb-icon-svg" viewBox="0 0 24 24" aria-hidden="true">
            <path className="tri-path" d="M12 2 L22 20 L2 20 Z" />
            <line className="tri-excl" x1="12" y1="10" x2="12" y2="15" />
            <circle className="tri-excl" cx="12" cy="17.5" r="0.5" />
          </svg>

          <span className="ceb-name">{this.props.name}</span>

          <span className={`ceb-msg${this.state.typed ? ' typed' : ''}`}>
            {this.state.error?.message ?? 'Unknown error'}
          </span>

          <button
            ref={this.retryRef}
            className="ceb-retry"
            onClick={this.handleRetry}
          >
            Try Again
          </button>

          {stack && (
            <>
              <button
                className="ceb-stack-toggle"
                aria-expanded={this.state.showStack}
                onClick={() => this.setState(s => ({ showStack: !s.showStack }))}
              >
                <span className="ceb-chevron">{'\u25B6'}</span> Stack Trace
              </button>
              <div className="ceb-stack-trace" data-open={this.state.showStack}>
                <div className="ceb-stack-trace-inner">
                  <pre>{renderStackTrace(stack)}</pre>
                </div>
              </div>
            </>
          )}

          {this.state.recovering && <div className="ceb-spinner" />}
        </div>
      );
    }
    return this.props.children;
  }
}
