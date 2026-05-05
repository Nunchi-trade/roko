import type { ReactNode } from 'react';
import './ScrollArea.css';

interface ScrollAreaProps {
  maxHeight?: string | number;
  className?: string;
  children: ReactNode;
}

export function ScrollArea({ maxHeight, className, children }: ScrollAreaProps) {
  const height = maxHeight == null
    ? undefined
    : typeof maxHeight === 'number'
      ? `${maxHeight}px`
      : maxHeight;

  return (
    <div
      className={`scroll-area${className ? ` ${className}` : ''}`}
      style={height ? { maxHeight: height } : undefined}
    >
      {children}
    </div>
  );
}
