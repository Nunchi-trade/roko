import './CostComparisonPanel.css';

export interface RunMetrics {
  cost: number;
  tokens: number;
  calls: number;
  elapsed: number;
}

export const EMPTY_RUN_METRICS: RunMetrics = { cost: 0, tokens: 0, calls: 0, elapsed: 0 };

interface CostComparisonPanelProps {
  naive: RunMetrics;
  cascade: RunMetrics;
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

export default function CostComparisonPanel({
  naive,
  cascade,
  isRunning = false,
}: CostComparisonPanelProps) {
  const hasBothCosts = naive.cost > 0 && cascade.cost > 0;
  const delta = hasBothCosts
    ? ((naive.cost - cascade.cost) / naive.cost) * 100
    : null;

  const hasData = naive.calls > 0 || cascade.calls > 0;
  const panelState = isRunning ? 'running' : hasData ? 'data' : 'pending';

  return (
    <section className="cost-panel" aria-label="Cost comparison">
      <div className="cost-panel-header">
        <span className="cost-panel-title">Cost Comparison</span>
        <span className={`cost-panel-live${hasData ? ' connected' : ''}`}>
          {panelState === 'pending' ? 'ready' : 'live'}
        </span>
      </div>

      <div className="cost-panel-columns">
        <div className={`cost-panel-column cost-panel-column--naive${isRunning && cascade.calls === 0 ? ' cost-panel-column--active' : ''}`}>
          <div className="cost-panel-column-label">Naive (no cascade)</div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Cost</span>
            <span className={`cost-panel-metric-value${naive.cost <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {fmtCost(naive.cost)}
            </span>
          </div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Tokens</span>
            <span className={`cost-panel-metric-value${naive.tokens <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {fmtTokens(naive.tokens)}
            </span>
          </div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Calls</span>
            <span className={`cost-panel-metric-value${naive.calls <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {naive.calls > 0 ? String(naive.calls) : '--'}
            </span>
          </div>
        </div>

        <div className={`cost-panel-column cost-panel-column--cascade${isRunning && cascade.calls > 0 ? ' cost-panel-column--active' : ''}`}>
          <div className="cost-panel-column-label">Cascade</div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Cost</span>
            <span className={`cost-panel-metric-value${cascade.cost <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {fmtCost(cascade.cost)}
            </span>
          </div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Tokens</span>
            <span className={`cost-panel-metric-value${cascade.tokens <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {fmtTokens(cascade.tokens)}
            </span>
          </div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Calls</span>
            <span className={`cost-panel-metric-value${cascade.calls <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {cascade.calls > 0 ? String(cascade.calls) : '--'}
            </span>
          </div>
        </div>
      </div>

      <div className="cost-panel-delta">
        <div className="cost-panel-delta-title">Savings</div>
        <div className={`cost-panel-delta-value${
          delta !== null ? (delta > 0 ? ' cost-panel-delta-value--savings' : ' cost-panel-delta-value--more-expensive') : ''
        }`}>
          {delta !== null ? `${delta > 0 ? '' : '+'}${Math.abs(delta).toFixed(0)}%` : '--'}
        </div>
        <div className="cost-panel-delta-label">
          {delta !== null
            ? delta > 0 ? 'cascade saved vs naive' : 'cascade cost more than naive'
            : 'run both to compare'}
        </div>
      </div>

      <div className="cost-panel-calls">
        <div className="cost-panel-calls-title">Inference Calls</div>
        <div className="cost-panel-calls-count">
          {naive.calls + cascade.calls} total ({naive.calls} naive, {cascade.calls} cascade)
        </div>
      </div>
    </section>
  );
}
