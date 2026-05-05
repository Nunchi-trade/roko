import type { ReactNode } from 'react';
import './Badge.css';

interface BadgeProps {
  children: ReactNode;
  variant?: 'default' | 'success' | 'error' | 'warning' | 'info';
}

export function Badge({ children, variant = 'default' }: BadgeProps) {
  return (
    <span className={`badge badge--${variant}`}>
      {children}
    </span>
  );
}
