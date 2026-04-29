import type { ReactNode } from 'react';
import './VerticalTimeline.css';

interface TimelineEntry {
  id: string;
  timestamp: string;
  title: string;
  description?: string;
  status?: 'success' | 'error' | 'warning' | 'info' | 'pending';
  icon?: ReactNode;
}

interface VerticalTimelineProps {
  entries: TimelineEntry[];
  maxHeight?: number | string;
  className?: string;
}

export function VerticalTimeline({
  entries,
  maxHeight,
  className,
}: VerticalTimelineProps) {
  const scrollable = maxHeight !== undefined;

  return (
    <div
      className={[
        'vtimeline',
        scrollable ? 'vtimeline--scrollable' : '',
        className,
      ]
        .filter(Boolean)
        .join(' ')}
      style={scrollable ? { maxHeight } : undefined}
    >
      {/* Vertical rail */}
      <div className="vtimeline__rail" />

      {entries.map((entry) => {
        const status = entry.status ?? 'info';

        return (
          <div key={entry.id} className="vtimeline__entry">
            {/* Timestamp */}
            <div className="vtimeline__timestamp">{entry.timestamp}</div>

            {/* Node on the rail */}
            <div className={`vtimeline__node vtimeline__node--${status}`} />

            {/* Card */}
            <div className="vtimeline__card">
              <div className="vtimeline__card-header">
                {entry.icon && (
                  <span className="vtimeline__card-icon">{entry.icon}</span>
                )}
                <span className="vtimeline__card-title">{entry.title}</span>
              </div>
              {entry.description && (
                <div className="vtimeline__card-description">
                  {entry.description}
                </div>
              )}
            </div>
          </div>
        );
      })}
    </div>
  );
}

export default VerticalTimeline;
