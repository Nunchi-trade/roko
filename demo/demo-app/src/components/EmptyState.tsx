import './EmptyState.css';

interface EmptyStateProps {
  title?: string;
  action?: { label: string; onClick: () => void };
}

export default function EmptyState({
  title = 'Nothing here yet',
  action,
}: EmptyStateProps) {
  return (
    <div className="empty-state">
      <div className="empty-state__icon" />
      <div className="empty-state__title">{title}</div>
      {action && (
        <button className="empty-state__action" onClick={action.onClick}>
          {action.label}
        </button>
      )}
    </div>
  );
}
