/**
 * DataSurface — A wrapper that shows loading/error/empty/data states.
 *
 * Standardizes the loading/error/empty pattern across all data-dependent panels.
 * Accepts state via props so it works with any data source (hooks, stores, fetch).
 *
 * Usage:
 *   <DataSurface loading={isLoading} error={errorMsg} empty={items.length === 0} onRetry={refetch}>
 *     <ItemList items={items} />
 *   </DataSurface>
 */
import type { ReactNode } from 'react';
import './DataSurface.css';

interface DataSurfaceProps {
  /** Whether data is still loading. Shows spinner. */
  loading?: boolean;
  /** Error message to display. Shows error state with optional retry. */
  error?: string | null;
  /** Whether the data set is empty (after loading). Shows empty state. */
  empty?: boolean;
  /** Label for the empty state message. */
  emptyLabel?: string;
  /** Called when user clicks "Retry" in error state. */
  onRetry?: () => void;
  /** The content to render when data is available. */
  children: ReactNode;
  /** Optional CSS class for the wrapper. */
  className?: string;
}

export default function DataSurface({
  loading,
  error,
  empty,
  emptyLabel = 'No data available',
  onRetry,
  children,
  className,
}: DataSurfaceProps) {
  const cls = `data-surface${className ? ` ${className}` : ''}`;

  if (loading) {
    return (
      <div className={cls}>
        <div className="ds-loading">
          <span className="ds-loading-label">Loading...</span>
        </div>
      </div>
    );
  }

  if (error) {
    return (
      <div className={cls}>
        <div className="ds-error">
          <span className="ds-error-msg">{error}</span>
          {onRetry && (
            <button className="ds-retry" onClick={onRetry} type="button">
              Retry
            </button>
          )}
        </div>
      </div>
    );
  }

  if (empty) {
    return (
      <div className={cls}>
        <div className="ds-empty">{emptyLabel}</div>
      </div>
    );
  }

  return <div className={cls}>{children}</div>;
}
