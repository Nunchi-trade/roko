import { useEffect, useRef, useState, type ReactNode } from 'react';
import './Cell.css';

interface CellProps {
  variant?: 'surface' | 'raised' | 'sunken' | 'glass';
  padding?: 'none' | 'sm' | 'md' | 'lg';
  glow?: 'none' | 'rose' | 'success' | 'error' | 'ambient';
  interactive?: boolean;
  selected?: boolean;
  flashOnChange?: boolean;
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
  flashOnChange,
  className,
  children,
  onClick,
}: CellProps) {
  const [flash, setFlash] = useState(false);
  const prevChildren = useRef(children);

  useEffect(() => {
    if (!flashOnChange) return;
    if (prevChildren.current !== children) {
      prevChildren.current = children;
      setFlash(true);
      const timer = setTimeout(() => setFlash(false), 400);
      return () => clearTimeout(timer);
    }
  }, [children, flashOnChange]);

  const cls = [
    'cell',
    `cell--${variant}`,
    `cell--pad-${padding}`,
    glow !== 'none' && `cell--glow-${glow}`,
    interactive && 'cell--interactive',
    selected && 'cell--selected',
    flash && 'cell--flash',
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
