import { Cell } from './Cell';
import { Badge } from '../design/Badge';

type PlanStatus = 'pending' | 'running' | 'done' | 'failed';
type CellStatus = 'idle' | 'active' | 'success' | 'error' | 'blocked';

const STATUS_MAP: Record<PlanStatus, CellStatus> = {
  pending: 'idle',
  running: 'active',
  done: 'success',
  failed: 'error',
};

interface PlanCellProps {
  plan: {
    id: string;
    title: string;
    taskCount: number;
    completedCount: number;
    status: PlanStatus;
    estimatedCost?: number;
  };
  onClick?: () => void;
}

function formatCost(cost: number): string {
  return cost < 0.01 ? `$${cost.toFixed(4)}` : `$${cost.toFixed(2)}`;
}

export function PlanCell({ plan, onClick }: PlanCellProps) {
  const pct = plan.taskCount > 0 ? plan.completedCount / plan.taskCount : 0;

  return (
    <Cell
      status={STATUS_MAP[plan.status]}
      identity="PLAN"
      onClick={onClick}
    >
      <div style={{ fontSize: 'var(--text-sm)', color: 'var(--text-primary)' }}>
        {plan.title}
      </div>

      {/* Progress bar */}
      <div style={{
        marginTop: 'var(--sp-2)',
        height: '4px',
        borderRadius: '2px',
        background: 'var(--glass-bg)',
        overflow: 'hidden',
      }}>
        <div style={{
          height: '100%',
          width: `${pct * 100}%`,
          background: 'var(--status-active)',
          borderRadius: '2px',
          transition: 'width 300ms var(--ease)',
        }} />
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--sp-1)', marginTop: 'var(--sp-2)', flexWrap: 'wrap' }}>
        <Badge>{plan.completedCount}/{plan.taskCount} tasks</Badge>
        {plan.estimatedCost != null && (
          <Badge>{formatCost(plan.estimatedCost)}</Badge>
        )}
      </div>
    </Cell>
  );
}
