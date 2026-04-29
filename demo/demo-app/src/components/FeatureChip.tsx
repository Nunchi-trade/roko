import type { ReactNode } from 'react';
import './FeatureChip.css';

interface FeatureChipProps {
  label: string;
  variant?: 'default' | 'accent' | 'dim' | 'success' | 'warning';
  size?: 'sm' | 'md';
  icon?: ReactNode;
  className?: string;
}

export function FeatureChip({
  label,
  variant = 'default',
  size = 'sm',
  icon,
  className,
}: FeatureChipProps) {
  return (
    <span
      className={`feature-chip feature-chip--${variant} feature-chip--${size}${className ? ` ${className}` : ''}`}
    >
      {icon && <span className="feature-chip__icon">{icon}</span>}
      {label}
    </span>
  );
}

interface FeatureChipGroupProps {
  children: ReactNode;
  className?: string;
}

export function FeatureChipGroup({ children, className }: FeatureChipGroupProps) {
  return (
    <span className={`feature-chip-group${className ? ` ${className}` : ''}`}>
      {children}
    </span>
  );
}
