import type { ReactNode } from 'react';
import './Cell.css';

type CellStatus = 'idle' | 'active' | 'success' | 'error' | 'blocked';

interface CellProps {
  status?: CellStatus;
  identity?: string;
  actions?: ReactNode;
  onClick?: () => void;
  selected?: boolean;
  className?: string;
  children: ReactNode;
}

export function Cell({
  status = 'idle',
  identity,
  actions,
  onClick,
  selected,
  className,
  children,
}: CellProps) {
  const classes = [
    'cell-container',
    `cell-container--${status}`,
    onClick && 'cell-container--clickable',
    selected && 'cell-container--selected',
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <div className={classes} onClick={onClick} role={onClick ? 'button' : undefined}>
      {identity && (
        <div className="cell-header">
          <span className="cell-header__led" />
          <span className="cell-header__identity">{identity}</span>
          <span className="cell-header__spacer" />
          {actions && <div className="cell-header__actions">{actions}</div>}
        </div>
      )}
      <div className="cell-body">{children}</div>
    </div>
  );
}
