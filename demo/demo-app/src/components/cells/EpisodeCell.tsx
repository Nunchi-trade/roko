import { Cell } from './Cell';
import { Badge } from '../design/Badge';

type EpisodeResult = 'pass' | 'fail';

interface EpisodeCellProps {
  episode: {
    hash: string;
    agent: string;
    result: EpisodeResult;
    cost?: number;
    duration?: number;
    model?: string;
  };
  onClick?: () => void;
}

function formatCost(cost: number): string {
  return cost < 0.01 ? `$${cost.toFixed(4)}` : `$${cost.toFixed(2)}`;
}

function formatDuration(ms: number): string {
  if (ms < 1000) return `${ms}ms`;
  return `${(ms / 1000).toFixed(1)}s`;
}

export function EpisodeCell({ episode, onClick }: EpisodeCellProps) {
  const cellStatus = episode.result === 'pass' ? 'success' as const : 'error' as const;
  const resultVariant = episode.result === 'pass' ? 'success' as const : 'error' as const;

  return (
    <Cell
      status={cellStatus}
      identity="EPISODE"
      onClick={onClick}
    >
      {/* Hash */}
      <div style={{
        fontFamily: 'var(--mono)',
        fontSize: 'var(--text-md)',
        color: 'var(--bone)',
        overflow: 'hidden',
        textOverflow: 'ellipsis',
        whiteSpace: 'nowrap',
      }}>
        {episode.hash.slice(0, 8)}
      </div>

      <div style={{ display: 'flex', alignItems: 'center', gap: 'var(--sp-1)', marginTop: 'var(--sp-2)', flexWrap: 'wrap' }}>
        <Badge>{episode.agent}</Badge>
        <Badge variant={resultVariant}>{episode.result}</Badge>
        {episode.cost != null && <Badge>{formatCost(episode.cost)}</Badge>}
        {episode.duration != null && <Badge>{formatDuration(episode.duration)}</Badge>}
        {episode.model && <Badge variant="info">{episode.model}</Badge>}
      </div>
    </Cell>
  );
}
