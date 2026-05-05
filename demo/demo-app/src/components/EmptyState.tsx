import { motion } from 'motion/react';
import { fadeUp } from '../design/motion-tokens';
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
    <motion.div
      className="empty-state"
      initial={fadeUp.initial}
      animate={fadeUp.animate}
      exit={fadeUp.exit}
    >
      <div className="empty-state__icon" />
      <div className="empty-state__title">{title}</div>
      {action && (
        <button className="empty-state__action" onClick={action.onClick}>
          {action.label}
        </button>
      )}
    </motion.div>
  );
}
