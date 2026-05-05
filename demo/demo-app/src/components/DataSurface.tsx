import type { ReactNode } from 'react';
import './DataSurface.css';

interface DataSurfaceProps {
  loading?: boolean;
  error?: Error | string | null;
  empty?: boolean;
  emptyIcon?: string;
  emptyTitle?: string;
  emptyAction?: { label: string; onClick: () => void };
  onRetry?: () => void;
  children: ReactNode;
}

function errorMessage(error: Error | string): string {
  if (typeof error === 'string') return error;
  return error.message || 'An unexpected error occurred';
}

export default function DataSurface({
  loading,
  error,
  empty,
  emptyTitle = 'No data yet',
  emptyAction,
  onRetry,
  children,
}: DataSurfaceProps) {
  if (loading) {
    return (
      <div className="data-surface">
        <div className="data-surface__loading">
          <div className="data-surface__skeleton-row skeleton" />
          <div className="data-surface__skeleton-row skeleton" />
          <div className="data-surface__skeleton-row skeleton" />
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className="data-surface">
        <div className="data-surface__error">
          <div className="data-surface__error-icon">!</div>
          <div className="data-surface__error-message">{errorMessage(error)}</div>
          {onRetry && (
            <button className="data-surface__retry-btn" onClick={onRetry}>
              Retry
            </button>
          )}
        </div>
      </div>
    );
  }

  if (empty) {
    return (
      <div className="data-surface">
        <div className="data-surface__empty">
          <div className="data-surface__empty-icon" />
          <div className="data-surface__empty-title">{emptyTitle}</div>
          {emptyAction && (
            <button className="data-surface__empty-action" onClick={emptyAction.onClick}>
              {emptyAction.label}
            </button>
          )}
        </div>
      </div>
    );
  }

  return (
    <div className="data-surface">
      <div className="data-surface__content">{children}</div>
    </div>
  );
}
