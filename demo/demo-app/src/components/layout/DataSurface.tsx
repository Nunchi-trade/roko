import type { ReactNode } from 'react';
import { Skeleton } from '../design/Skeleton';
import { EmptyState } from '../design/EmptyState';
import { ErrorState } from '../design/ErrorState';
import './DataSurface.css';

interface DataSurfaceProps<T> {
  data: T | null | undefined;
  loading?: boolean;
  error?: string | null;
  empty?: boolean;
  emptyMessage?: string;
  emptyAction?: string;
  onRetry?: () => void;
  children: (data: T) => ReactNode;
  className?: string;
  minHeight?: string | number;
}

export function DataSurface<T>({
  data,
  loading,
  error,
  empty,
  emptyMessage = 'No data',
  emptyAction,
  onRetry,
  children,
  className,
  minHeight = 200,
}: DataSurfaceProps<T>) {
  const height = typeof minHeight === 'number' ? `${minHeight}px` : minHeight;
  const cls = `data-surface${className ? ` ${className}` : ''}`;

  // Loading state (only when no data yet)
  if (loading && !data) {
    return (
      <div className={`${cls} data-surface--centered`} style={{ minHeight: height }}>
        <Skeleton variant="pane" />
      </div>
    );
  }

  // Error state
  if (error) {
    return (
      <div className={`${cls} data-surface--centered`} style={{ minHeight: height }}>
        <ErrorState message={error} onRetry={onRetry} />
      </div>
    );
  }

  // Empty state
  if (!data || empty || (Array.isArray(data) && data.length === 0)) {
    return (
      <div className={`${cls} data-surface--centered`} style={{ minHeight: height }}>
        <EmptyState message={emptyMessage} action={emptyAction} />
      </div>
    );
  }

  // Content
  return (
    <div className={cls} style={{ minHeight: height }}>
      {children(data)}
    </div>
  );
}
