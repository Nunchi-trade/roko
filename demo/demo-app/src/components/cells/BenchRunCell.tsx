import { Cell } from './Cell';
import { Badge } from '../design/Badge';
import { StatusBadge } from '../design/StatusBadge';

type BenchStatus = 'pending' | 'running' | 'done' | 'failed';
type CellStatus = 'idle' | 'active' | 'success' | 'error' | 'blocked';

const STATUS_MAP: Record<BenchStatus, CellStatus> = {
  pending: 'idle',
  running: 'active',
  done: 'success',
  failed: 'error',
};

const BADGE_STATUS_MAP: Record<BenchStatus, 'idle' | 'active' | 'success' | 'error'> = {
  pending: 'idle',
  running: 'active',
  done: 'success',
  failed: 'error',
};

interface BenchRunCellProps {
  run: {
    id: string;
    suite: string;
    model: string;
    passRate: number;
    cost: number;
    status: BenchStatus;
  };
  onClick?: () => void;
}

function formatCost(cost: number): string {
  return cost < 0.01 ? `$${cost.toFixed(4)}` : `$${cost.toFixed(2)}`;
}

export function BenchRunCell({ run, onClick }: BenchRunCellProps) {
  const pctDisplay = `${Math.round(run.passRate * 100)}%`;

  return (
    <Cell
      status={STATUS_MAP[run.status]}
      identity="BENCH"
      actions={<StatusBadge status={BADGE_STATUS_MAP[run.status]} />}
      onClick={onClick}
    >
      {/* Suite name */}
      <div style={{ fontSize: 'var(--text-sm)', fontWeight: 600, color: 'var(--text-primary)' }}>
        {run.suite}
      </div>

      {/* Model badge */}
      <div style={{ marginTop: 'var(--sp-1)' }}>
        <Badge variant="info">{run.model}</Badge>
      </div>

      {/* Pass rate + cost row */}
      <div style={{
        display: 'flex',
        alignItems: 'baseline',
        justifyContent: 'space-between',
        marginTop: 'var(--sp-2)',
      }}>
        <span style={{
          fontFamily: 'var(--mono)',
          fontSize: 'var(--text-xl)',
          fontWeight: 500,
          color: 'var(--bone-bright)',
          letterSpacing: '-0.01em',
          lineHeight: 1,
        }}>
          {pctDisplay}
        </span>
        <Badge>{formatCost(run.cost)}</Badge>
      </div>
    </Cell>
  );
}
