import { Component, createRef } from 'react';
import type { ErrorInfo, ReactNode } from 'react';

interface Props {
  children: ReactNode;
}

interface State {
  hasError: boolean;
  error: Error | null;
  recovering: boolean;
}

export default class ErrorBoundary extends Component<Props, State> {
  state: State = { hasError: false, error: null, recovering: false };
  private btnRef = createRef<HTMLButtonElement>();

  static getDerivedStateFromError(error: Error): Partial<State> {
    return { hasError: true, error };
  }

  componentDidCatch(error: Error, info: ErrorInfo) {
    console.error('ErrorBoundary caught:', error, info.componentStack);
  }

  private handleRetry = () => {
    const btn = this.btnRef.current;
    if (btn) btn.classList.add('eb-pulsing');
    this.setState({ recovering: true });
    setTimeout(() => {
      this.setState({ hasError: false, error: null, recovering: false });
    }, 450);
  };

  render() {
    if (this.state.hasError) {
      return (
        <div style={{
          position: 'fixed', inset: 0,
          display: 'flex', flexDirection: 'column',
          alignItems: 'center', justifyContent: 'center',
          background: 'var(--bg-void, #060608)',
          color: 'var(--text-soft, #a89098)',
          fontFamily: 'var(--mono, monospace)',
          animation: this.state.recovering
            ? 'eb-dissolve 0.4s cubic-bezier(0.22,1,0.36,1) forwards'
            : 'eb-enter 0.6s cubic-bezier(0.22,1,0.36,1) both',
        }}>
          <style>{`
            @keyframes eb-enter {
              0% { opacity: 0; transform: translateX(-4px) skewX(-1deg); }
              8% { opacity: 1; transform: translateX(6px) skewX(1.5deg); }
              16% { transform: translateX(-3px) skewX(-0.8deg); }
              24% { transform: translateX(2px) skewX(0.4deg); }
              30% { transform: translateX(0) skewX(0); }
              100% { opacity: 1; transform: none; }
            }
            @keyframes eb-dissolve {
              to { opacity: 0; transform: scale(0.92); }
            }
            @keyframes eb-draw { to { stroke-dashoffset: 0; } }
            @keyframes eb-type {
              from { max-width: 0; }
              to { max-width: 320px; }
            }
            @keyframes eb-blink { 50% { border-right-color: transparent; } }
            @keyframes eb-fade-up {
              from { opacity: 0; transform: translateY(8px); }
              to { opacity: 1; transform: translateY(0); }
            }
            @keyframes eb-pulse-ring {
              0% { transform: scale(1); opacity: 0.8; }
              100% { transform: scale(1.6); opacity: 0; }
            }
            @keyframes eb-spin { to { transform: rotate(360deg); } }
            .eb-pulsing::after {
              animation: eb-pulse-ring 0.5s cubic-bezier(0.22,1,0.36,1) forwards !important;
              opacity: 1 !important;
            }
          `}</style>

          {/* animated warning triangle */}
          <svg
            width="36" height="36" viewBox="0 0 24 24"
            style={{ animation: 'eb-fade-up 0.4s 0.3s cubic-bezier(0.22,1,0.36,1) both' }}
          >
            <path
              d="M12 2 L22 20 L2 20 Z"
              stroke="var(--status-error, #fb7185)"
              strokeWidth="2" strokeLinecap="round" strokeLinejoin="round"
              fill="none"
              style={{ strokeDasharray: 120, strokeDashoffset: 120, animation: 'eb-draw 0.8s 0.35s cubic-bezier(0.22,1,0.36,1) forwards' }}
            />
            <line
              x1="12" y1="10" x2="12" y2="15"
              stroke="var(--status-error, #fb7185)"
              strokeWidth="2" strokeLinecap="round"
              style={{ strokeDasharray: 12, strokeDashoffset: 12, animation: 'eb-draw 0.3s 0.9s cubic-bezier(0.22,1,0.36,1) forwards' }}
            />
            <circle cx="12" cy="17.5" r="0.5"
              stroke="var(--status-error, #fb7185)"
              strokeWidth="2" fill="none"
              style={{ strokeDasharray: 12, strokeDashoffset: 12, animation: 'eb-draw 0.3s 0.9s cubic-bezier(0.22,1,0.36,1) forwards' }}
            />
          </svg>

          {/* typewriter message */}
          <div style={{
            fontSize: 'var(--text-lg)', letterSpacing: '.12em', marginTop: 'var(--sp-4)', marginBottom: 'var(--sp-4)',
            overflow: 'hidden', whiteSpace: 'nowrap',
            borderRight: '2px solid var(--status-error, #fb7185)',
            animation: 'eb-type 1.2s 0.5s steps(40, end) both, eb-blink 0.6s 0.5s step-end 4',
          }}>
            {this.state.error?.message ?? 'Something went wrong'}
          </div>

          <button
            ref={this.btnRef}
            onClick={this.handleRetry}
            style={{
              padding: 'var(--sp-2) var(--sp-5)',
              fontSize: 'var(--text-md)',
              letterSpacing: '.2em',
              textTransform: 'uppercase',
              color: 'var(--status-error, #fb7185)',
              background: 'transparent',
              border: '1px solid rgba(251,113,133,.3)',
              cursor: 'pointer',
              fontFamily: 'inherit',
              borderRadius: 3,
              position: 'relative',
              overflow: 'hidden',
              transition: 'border-color 150ms, box-shadow 150ms, color 150ms',
              animation: 'eb-fade-up 0.4s 0.7s cubic-bezier(0.22,1,0.36,1) both',
            }}
            onMouseEnter={e => {
              const t = e.currentTarget;
              t.style.borderColor = 'var(--status-error, #fb7185)';
              t.style.boxShadow = '0 0 16px rgba(251,113,133,.3)';
              t.style.color = 'var(--text-strong, #f8f0f8)';
            }}
            onMouseLeave={e => {
              const t = e.currentTarget;
              t.style.borderColor = 'rgba(251,113,133,.3)';
              t.style.boxShadow = 'none';
              t.style.color = 'var(--status-error, #fb7185)';
            }}
          >
            Try Again
          </button>

          {this.state.recovering && (
            <div style={{
              width: 20, height: 20, marginTop: 16,
              border: '2px solid rgba(255,255,255,.1)',
              borderTopColor: 'var(--status-error, #fb7185)',
              borderRadius: '50%',
              animation: 'eb-spin 0.6s linear infinite',
            }} />
          )}
        </div>
      );
    }
    return this.props.children;
  }
}
