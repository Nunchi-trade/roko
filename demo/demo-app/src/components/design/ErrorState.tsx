import './ErrorState.css';

interface ErrorStateProps {
  message: string;
  details?: string;
  onRetry?: () => void;
}

export function ErrorState({ message, details, onRetry }: ErrorStateProps) {
  return (
    <div className="error-state">
      <span className="error-state__icon">{'\u2715'}</span>
      <div className="error-state__message">{message}</div>

      {details && (
        <details className="error-state__details">
          <summary className="error-state__details-toggle">Details</summary>
          <div className="error-state__details-content">{details}</div>
        </details>
      )}

      {onRetry && (
        <button className="error-state__retry" onClick={onRetry} type="button">
          Retry
        </button>
      )}
    </div>
  );
}
