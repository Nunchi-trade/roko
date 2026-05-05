import { Children, type CSSProperties, type ReactNode } from 'react';
import './CellGrid.css';

interface CellGridProps {
  minWidth?: number;
  gap?: string;
  className?: string;
  children: ReactNode;
}

export function CellGrid({
  minWidth = 280,
  gap = '8px',
  className,
  children,
}: CellGridProps) {
  const style = {
    '--cell-grid-min': `${minWidth}px`,
    gap,
  } as CSSProperties;

  const classes = ['cell-grid', className].filter(Boolean).join(' ');

  return (
    <div className={classes} style={style}>
      {Children.map(children, (child, i) => (
        <div className="cell-grid__item" style={{ '--i': i } as CSSProperties}>
          {child}
        </div>
      ))}
    </div>
  );
}
