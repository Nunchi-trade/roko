import type { ReactNode } from 'react';
import { motion } from 'motion/react';
import { slideRight } from '../../design/motion-tokens';
import './CellDetail.css';

/* ── Sub-components ── */

function Section({ title, children }: { title: string; children: ReactNode }) {
  return (
    <div className="cell-detail__section">
      {title && <div className="cell-detail__section-title">{title}</div>}
      {children}
    </div>
  );
}

function Field({ label, value, mono }: { label: string; value: ReactNode; mono?: boolean }) {
  return (
    <div className="cell-detail__field">
      <span className="cell-detail__field-label">{label}</span>
      <span className={`cell-detail__field-value${mono ? ' cell-detail__field-value--mono' : ''}`}>
        {value}
      </span>
    </div>
  );
}

function Divider() {
  return <div className="cell-detail__divider" />;
}

/* ── Main component ── */

type DetailStatus = 'active' | 'idle' | 'error' | 'completed';

interface CellDetailAction {
  label: string;
  onClick: () => void;
  variant?: 'primary' | 'ghost';
}

interface CellDetailProps {
  title: string;
  subtitle?: string;
  icon?: ReactNode;
  status?: DetailStatus;
  actions?: CellDetailAction[];
  children: ReactNode;
}

function CellDetail({ title, subtitle, icon, status, actions, children }: CellDetailProps) {
  return (
    <motion.aside
      className="cell-detail"
      initial={slideRight.initial}
      animate={slideRight.animate}
      exit={slideRight.exit}
    >
      <div className="cell-detail__header">
        {icon && <div className="cell-detail__icon">{icon}</div>}
        <div className="cell-detail__titles">
          <div className="cell-detail__title">{title}</div>
          {subtitle && <div className="cell-detail__subtitle">{subtitle}</div>}
        </div>
        {status && (
          <span className={`cell-detail__status cell-detail__status--${status}`}>
            {status}
          </span>
        )}
        {actions && actions.length > 0 && (
          <div className="cell-detail__actions">
            {actions.map((a) => (
              <button
                key={a.label}
                className={`cell-detail__action cell-detail__action--${a.variant ?? 'ghost'}`}
                onClick={a.onClick}
              >
                {a.label}
              </button>
            ))}
          </div>
        )}
      </div>
      <div className="cell-detail__body">{children}</div>
    </motion.aside>
  );
}

CellDetail.Section = Section;
CellDetail.Field = Field;
CellDetail.Divider = Divider;

export { CellDetail };
export type { CellDetailProps, CellDetailAction, DetailStatus };
