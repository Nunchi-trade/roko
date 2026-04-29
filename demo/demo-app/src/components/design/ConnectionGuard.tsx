import { useRef, useState, useEffect, type ReactNode } from 'react';
import './ConnectionGuard.css';

interface ConnectionGuardProps {
  connected: boolean;
  connecting: boolean;
  error?: string;
  children: ReactNode;
  onRetry?: () => void;
  retryIn?: number;
  className?: string;
}

const BRAILLE_FRAMES = ['\u2840', '\u2844', '\u2846', '\u2807', '\u2823', '\u2831', '\u2838', '\u2834'];

export default function ConnectionGuard({
  connected,
  connecting,
  error,
  children,
  onRetry,
  retryIn,
  className,
}: ConnectionGuardProps) {
  const frameRef = useRef(0);
  const [spinChar, setSpinChar] = useState(BRAILLE_FRAMES[0]);

  // Braille spinner animation
  useEffect(() => {
    if (!connecting) return;

    const id = setInterval(() => {
      frameRef.current = (frameRef.current + 1) % BRAILLE_FRAMES.length;
      setSpinChar(BRAILLE_FRAMES[frameRef.current]);
    }, 100);

    return () => clearInterval(id);
  }, [connecting]);

  // Connecting state
  if (connecting) {
    return (
      <div className={`connection-guard${className ? ` ${className}` : ''}`}>
        <div className="connection-guard__connecting">
          <span className="connection-guard__spinner">{spinChar}</span>
          <span className="connection-guard__label">ESTABLISHING LINK...</span>
        </div>
      </div>
    );
  }

  // Error state
  if (!connected && error) {
    return (
      <div className={`connection-guard${className ? ` ${className}` : ''}`}>
        <div className="connection-guard__error">
          <span className="connection-guard__error-icon">{'\u2715'}</span>
          <div className="connection-guard__error-msg">{error}</div>
          {onRetry && (
            <button className="connection-guard__retry" onClick={onRetry} type="button">
              Retry
            </button>
          )}
          {retryIn !== undefined && retryIn > 0 && (
            <span className="connection-guard__countdown">
              RETRY IN {retryIn}s
            </span>
          )}
        </div>
      </div>
    );
  }

  // Connected: render children
  if (connected) {
    return (
      <div className={`connection-guard${className ? ` ${className}` : ''}`}>
        <div className="connection-guard__content">{children}</div>
      </div>
    );
  }

  // Fallback: not connected, not connecting, no error
  return (
    <div className={`connection-guard${className ? ` ${className}` : ''}`}>
      <div className="connection-guard__connecting">
        <span className="connection-guard__label">WAITING FOR CONNECTION...</span>
      </div>
    </div>
  );
}
