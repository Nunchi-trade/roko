import { useEventStreamContext } from '../contexts/EventStreamContext';
import { useInferenceCosts } from '../hooks/useOperationEvents';
import './CostComparisonPanel.css';

interface CostComparisonPanelProps {
  /** Operation ID for the naive (--no-cascade) run */
  naiveOpId?: string | null;
  /** Operation ID for the cascade-routed run */
  cascadeOpId?: string | null;
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
  naiveOpId = null,
  cascadeOpId = null,
  isRunning = false,
}: CostComparisonPanelProps) {
  const { connected } = useEventStreamContext();
  const naiveCosts = useInferenceCosts(naiveOpId);
  const cascadeCosts = useInferenceCosts(cascadeOpId);

  const hasBothCosts = naiveCosts.totalCost > 0 && cascadeCosts.totalCost > 0;
  const delta = hasBothCosts
    ? ((naiveCosts.totalCost - cascadeCosts.totalCost) / naiveCosts.totalCost) * 100
    : null;

  const panelState = isRunning ? 'running' : naiveCosts.calls > 0 || cascadeCosts.calls > 0 ? 'data' : 'pending';

  return (
    <section className="cost-panel" aria-label="Cost comparison">
      <div className="cost-panel-header">
        <span className="cost-panel-title">Cost Comparison</span>
        <span className={`cost-panel-live ${connected ? 'connected' : ''}`}>
          {panelState === 'pending' ? (connected ? 'armed' : 'offline') : 'live'}
        </span>
      </div>

      <div className="cost-panel-columns">
        <div className={`cost-panel-column cost-panel-column--naive${isRunning && !cascadeOpId ? ' cost-panel-column--active' : ''}`}>
          <div className="cost-panel-column-label">Naive (no cascade)</div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Cost</span>
            <span className={`cost-panel-metric-value${naiveCosts.totalCost <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {fmtCost(naiveCosts.totalCost)}
            </span>
          </div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Tokens</span>
            <span className={`cost-panel-metric-value${naiveCosts.totalTokens <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {fmtTokens(naiveCosts.totalTokens)}
            </span>
          </div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Calls</span>
            <span className={`cost-panel-metric-value${naiveCosts.calls <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {naiveCosts.calls > 0 ? String(naiveCosts.calls) : '--'}
            </span>
          </div>
        </div>

        <div className={`cost-panel-column cost-panel-column--cascade${isRunning && cascadeOpId ? ' cost-panel-column--active' : ''}`}>
          <div className="cost-panel-column-label">Cascade</div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Cost</span>
            <span className={`cost-panel-metric-value${cascadeCosts.totalCost <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {fmtCost(cascadeCosts.totalCost)}
            </span>
          </div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Tokens</span>
            <span className={`cost-panel-metric-value${cascadeCosts.totalTokens <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {fmtTokens(cascadeCosts.totalTokens)}
            </span>
          </div>
          <div className="cost-panel-metric">
            <span className="cost-panel-metric-label">Calls</span>
            <span className={`cost-panel-metric-value${cascadeCosts.calls <= 0 ? ' cost-panel-metric-value--empty' : ''}`}>
              {cascadeCosts.calls > 0 ? String(cascadeCosts.calls) : '--'}
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
          {naiveCosts.calls + cascadeCosts.calls} total ({naiveCosts.calls} naive, {cascadeCosts.calls} cascade)
        </div>
      </div>
    </section>
  );
}
