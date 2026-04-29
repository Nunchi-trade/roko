import type { ReactNode } from 'react';
import './Cell.css';

interface CellProps {
  variant?: 'surface' | 'raised' | 'sunken' | 'glass';
  padding?: 'none' | 'sm' | 'md' | 'lg';
  glow?: 'none' | 'rose' | 'success' | 'error' | 'ambient';
  interactive?: boolean;
  selected?: boolean;
  className?: string;
  children: ReactNode;
  onClick?: () => void;
}

export function Cell({
  variant = 'surface',
  padding = 'md',
  glow = 'none',
  interactive,
  selected,
  className,
  children,
  onClick,
}: CellProps) {
  const cls = [
    'cell',
    `cell--${variant}`,
    `cell--pad-${padding}`,
    glow !== 'none' && `cell--glow-${glow}`,
    interactive && 'cell--interactive',
    selected && 'cell--selected',
    className,
  ]
    .filter(Boolean)
    .join(' ');

  return (
    <div
      className={cls}
      onClick={onClick}
      role={onClick ? 'button' : undefined}
      tabIndex={onClick ? 0 : undefined}
    >
      {children}
    </div>
  );
}
