import type { ReactNode } from 'react';
import './CellTimeline.css';

interface CellTimelineItemProps {
  timestamp?: string;
  icon?: ReactNode;
  status?: 'active' | 'done' | 'error' | 'pending';
  children: ReactNode;
}

function CellTimelineItem({
  timestamp,
  icon,
  status = 'pending',
  children,
}: CellTimelineItemProps) {
  return (
    <div className={`cell-timeline-item cell-timeline-item--${status}`}>
      <span className="cell-timeline-item__dot" />
      {(timestamp || icon) && (
        <div className="cell-timeline-item__header">
          {icon && <span className="cell-timeline-item__icon">{icon}</span>}
          {timestamp && (
            <span className="cell-timeline-item__timestamp">{timestamp}</span>
          )}
        </div>
      )}
      <div className="cell-timeline-item__content">{children}</div>
    </div>
  );
}

interface CellTimelineProps {
  children: ReactNode;
  className?: string;
}

export function CellTimeline({ children, className }: CellTimelineProps) {
  const cls = ['cell-timeline', className].filter(Boolean).join(' ');
  return <div className={cls}>{children}</div>;
}

CellTimeline.Item = CellTimelineItem;
