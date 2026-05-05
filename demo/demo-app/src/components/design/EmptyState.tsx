import './EmptyState.css';

interface EmptyStateProps {
  message: string;
  action?: string;
  hint?: string;
}

export function EmptyState({ message, action, hint }: EmptyStateProps) {
  return (
    <div className="empty-state">
      <div className="empty-state__message">{message}</div>
      {action && <div className="empty-state__action">{action}</div>}
      {hint && <div className="empty-state__hint">{hint}</div>}
    </div>
  );
}
