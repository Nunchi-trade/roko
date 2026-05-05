import { useMemo } from 'react';
import type { RunMetrics } from './CostComparisonPanel';
import './MemoryTransferPanel.css';

interface MemoryTransferPanelProps {
  cold: RunMetrics;
  warm: RunMetrics;
  isRunning?: boolean;
}

function fmtCost(n: number): string {
  if (n <= 0) return '--';
  if (n < 0.001) return `$${n.toFixed(4)}`;
  if (n < 0.01) return `$${n.toFixed(3)}`;
  return `$${n.toFixed(2)}`;
}

function fmtTokens(n: number): string {
  if (n <= 0) return '--';
  return n >= 1000 ? `${(n / 1000).toFixed(1)}k` : String(n);
}

export default function MemoryTransferPanel({
  cold,
  warm,
  isRunning = false,
}: MemoryTransferPanelProps) {
  const hasBothCosts = cold.cost > 0 && warm.cost > 0;
  const costDelta = hasBothCosts
    ? ((cold.cost - warm.cost) / cold.cost) * 100
    : null;

  type Phase = 'cold' | 'ingest' | 'warm' | 'delta';
  const currentPhase: Phase = useMemo(() => {
    if (warm.calls > 0) return hasBothCosts ? 'delta' : 'warm';
    if (cold.calls > 0 && cold.cost > 0) return 'ingest';
    if (cold.calls > 0) return 'cold';
    return 'cold';
  }, [cold.calls, cold.cost, warm.calls, hasBothCosts]);

  const phases: { id: Phase; label: string }[] = [
    { id: 'cold', label: 'Cold' },
    { id: 'ingest', label: 'Ingest' },
    { id: 'warm', label: 'Warm' },
    { id: 'delta', label: 'Delta' },
  ];

  const phaseOrder: Phase[] = ['cold', 'ingest', 'warm', 'delta'];
  const currentIdx = phaseOrder.indexOf(currentPhase);

  const hasData = cold.calls > 0 || warm.calls > 0;
  const panelState = isRunning ? 'running' : hasData ? 'data' : 'pending';

  return (
    <section className="memory-panel" aria-label="Memory transfer">
      <div className="memory-panel-header">
        <span className="memory-panel-title">Knowledge Transfer</span>
        <span className={`memory-panel-live${hasData ? ' connected' : ''}`}>
          {panelState === 'pending' ? 'ready' : 'live'}
        </span>
      </div>

      <div className="memory-panel-phases">
        {phases.map((phase, i) => {
          const status = i < currentIdx ? 'done' : i === currentIdx && (isRunning || cold.calls > 0) ? 'active' : '';
          return (
            <div key={phase.id} className={`memory-panel-phase${status ? ` memory-panel-phase--${status}` : ''}`}>
              <div className="memory-panel-phase-dot" />
              <div className="memory-panel-phase-label">{phase.label}</div>
            </div>
          );
        })}
      </div>

      <div className="memory-panel-columns">
        <div className="memory-panel-column memory-panel-column--cold">
          <div className="memory-panel-column-label">Cold Run</div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Cost</span>
            <span className={`memory-panel-metric-value${cold.cost <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {fmtCost(cold.cost)}
            </span>
          </div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Tokens</span>
            <span className={`memory-panel-metric-value${cold.tokens <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {fmtTokens(cold.tokens)}
            </span>
          </div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Calls</span>
            <span className={`memory-panel-metric-value${cold.calls <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {cold.calls > 0 ? String(cold.calls) : '--'}
            </span>
          </div>
        </div>

        <div className="memory-panel-column memory-panel-column--warm">
          <div className="memory-panel-column-label">Warm Run</div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Cost</span>
            <span className={`memory-panel-metric-value${warm.cost <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {fmtCost(warm.cost)}
            </span>
          </div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Tokens</span>
            <span className={`memory-panel-metric-value${warm.tokens <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {fmtTokens(warm.tokens)}
            </span>
          </div>
          <div className="memory-panel-metric">
            <span className="memory-panel-metric-label">Calls</span>
            <span className={`memory-panel-metric-value${warm.calls <= 0 ? ' memory-panel-metric-value--empty' : ''}`}>
              {warm.calls > 0 ? String(warm.calls) : '--'}
            </span>
          </div>
        </div>
      </div>

      <div className="memory-panel-transfer">
        <div className="memory-panel-transfer-title">Efficiency Gain</div>
        <div className={`memory-panel-transfer-value${
          costDelta !== null && costDelta > 0 ? ' memory-panel-transfer-value--savings' : ''
        }`}>
          {costDelta !== null ? `${costDelta > 0 ? '' : '+'}${Math.abs(costDelta).toFixed(0)}%` : '--'}
        </div>
        <div className="memory-panel-transfer-label">
          {costDelta !== null
            ? costDelta > 0 ? 'warm run cheaper via knowledge reuse' : 'warm run needed more work'
            : 'run both to compare'}
        </div>
      </div>
    </section>
  );
}
