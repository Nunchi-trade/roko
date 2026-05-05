import type { CSSProperties, ReactNode } from 'react';
import './CellGrid.css';

interface CellGridProps {
  columns?: number | 'auto';
  gap?: 'sm' | 'md' | 'lg';
  className?: string;
  children: ReactNode;
}

export function CellGrid({
  columns = 'auto',
  gap = 'md',
  className,
  children,
}: CellGridProps) {
  const style: CSSProperties =
    columns !== 'auto'
      ? { gridTemplateColumns: `repeat(${columns}, 1fr)` }
      : {};

  const cls = [
    'cell-grid',
    columns === 'auto' && 'cell-grid--auto',
    `cell-grid--gap-${gap}`,
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <div className={cls} style={style}>
      {children}
    </div>
  );
}
