import './StatusBadge.css';

type Status = 'idle' | 'active' | 'success' | 'error' | 'blocked' | 'warning';

interface StatusBadgeProps {
  status: Status;
  label?: string;
  size?: 'sm' | 'md';
}

const STATUS_ICONS: Record<Status, string> = {
  idle: '\u25CB',       // ○
  active: '\u25C9',     // ◉
  success: '\u2713',    // ✓
  error: '\u2715',      // ✕
  blocked: '\u2B21',    // ⬡
  warning: '\u26A0',    // ⚠
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
      <span className="status-badge__icon">{STATUS_ICONS[status]}</span>
      <span className="status-badge__label">{displayLabel}</span>
    </span>
  );
}
