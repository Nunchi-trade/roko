import type { ReactNode } from 'react';
import { CheckmarkIcon, CrossIcon, SpinnerIcon, PulseIcon, WarningIcon } from '../icons/AnimatedIcons';
import './StatusBadge.css';

type Status = 'idle' | 'active' | 'success' | 'error' | 'blocked' | 'warning';

interface StatusBadgeProps {
  status: Status;
  label?: string;
  size?: 'sm' | 'md';
}

const STATUS_ANIMATED_ICONS: Record<Status, ReactNode> = {
  idle: <PulseIcon size={12} color="var(--text-muted)" />,
  active: <SpinnerIcon size={12} color="var(--bone-bright)" />,
  success: <CheckmarkIcon size={12} color="var(--success)" />,
  error: <CrossIcon size={12} color="var(--rose-bright)" />,
  blocked: <WarningIcon size={12} color="var(--warning)" />,
  warning: <WarningIcon size={12} color="var(--warning)" />,
};

const STATUS_LABELS: Record<Status, string> = {
  idle: 'Idle',
  active: 'Active',
  success: 'Success',
  error: 'Error',
  blocked: 'Blocked',
  warning: 'Warning',
};

export function StatusBadge({ status, label, size = 'sm' }: StatusBadgeProps) {
  const displayLabel = label ?? STATUS_LABELS[status];

  return (
    <span
      className={`status-badge status-badge--${size} status-badge--${status}`}
    >
      <span className="status-badge__dot" />
      <span className="status-badge__icon">{STATUS_ANIMATED_ICONS[status]}</span>
      <span className="status-badge__label">{displayLabel}</span>
    </span>
  );
}
