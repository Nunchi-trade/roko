import { useRef, useEffect, useState, type ReactNode } from 'react';
import { Skeleton } from './Skeleton';
import './LazyPane.css';

type ConnectionState = 'connecting' | 'connected' | 'error' | 'disconnected';

interface LazyPaneProps {
  connectionState: ConnectionState;
  hasData: boolean;
  label?: string;
  children: ReactNode;
  onReconnect?: () => void;
  retryCountdown?: number;
  skeleton?: ReactNode;
  className?: string;
}

export default function LazyPane({
  connectionState,
  hasData,
  label,
  children,
  onReconnect,
  retryCountdown,
  skeleton,
  className,
}: LazyPaneProps) {
  const prevStateRef = useRef(connectionState);
  const [flash, setFlash] = useState(false);

  // Flash border on state change
  useEffect(() => {
    if (prevStateRef.current !== connectionState) {
      prevStateRef.current = connectionState;
      setFlash(true);
      const timer = setTimeout(() => setFlash(false), 400);
      return () => clearTimeout(timer);
    }
  }, [connectionState]);

  const ledClass = `lazy-pane__led lazy-pane__led--${connectionState}`;

  const statusText = (): string => {
    if (connectionState === 'connecting') return 'CONNECTING...';
    if (connectionState === 'connected' && !hasData) return 'AWAITING DATA...';
    if (connectionState === 'error') return 'ERROR';
    if (connectionState === 'disconnected') return 'DISCONNECTED';
    return label ?? 'CONNECTED';
  };

  const renderSkeleton = () => (
    <div className="lazy-pane__skeleton">
      {skeleton ?? <Skeleton variant="pane" />}
    </div>
  );

  const renderError = () => (
    <div className="lazy-pane__error">
      <span className="lazy-pane__error-icon">{'\u2715'}</span>
      <div className="lazy-pane__error-msg">Connection failed</div>
      {onReconnect && (
        <button className="lazy-pane__reconnect" onClick={onReconnect} type="button">
          Reconnect
        </button>
      )}
      {retryCountdown !== undefined && retryCountdown > 0 && (
        <span className="lazy-pane__countdown">
          RETRY IN {retryCountdown}s
        </span>
      )}
    </div>
  );

  const renderBody = () => {
    switch (connectionState) {
      case 'connecting':
        return renderSkeleton();

      case 'connected':
        if (!hasData) return renderSkeleton();
        return <div className="lazy-pane__content">{children}</div>;

      case 'error':
        return renderError();

      case 'disconnected':
        return (
          <>
            <div className="lazy-pane__banner">
              <span className={ledClass} />
              CONNECTION LOST
            </div>
            <div className="lazy-pane__content lazy-pane__content--dimmed">
              {children}
            </div>
          </>
        );
    }
  };

  return (
    <div
      className={[
        'lazy-pane',
        flash ? 'lazy-pane--flash' : '',
        className,
      ].filter(Boolean).join(' ')}
    >
      {connectionState !== 'disconnected' && (
        <div className="lazy-pane__status">
          <span className={ledClass} />
          {statusText()}
        </div>
      )}
      {renderBody()}
    </div>
  );
}
