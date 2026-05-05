import type { RunMetrics } from './CostComparisonPanel';
import './OracleFlowPanel.css';

interface OracleFlowPanelProps {
  data: RunMetrics;
  strategy: RunMetrics;
  chainChecked?: boolean;
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

type FlowStep = 'connect' | 'scan' | 'write' | 'recommend';

const FLOW_STEPS: { id: FlowStep; label: string; desc: string; icon: string }[] = [
  { id: 'connect', label: 'Connect', desc: 'Verify local Ethereum fork', icon: '1' },
  { id: 'scan', label: 'Scan', desc: 'Query DeFi lending rates', icon: '2' },
  { id: 'write', label: 'Write', desc: 'Store analysis to knowledge', icon: '3' },
  { id: 'recommend', label: 'Recommend', desc: 'USDC allocation strategy', icon: '4' },
];

export default function OracleFlowPanel({
  data,
  strategy,
  chainChecked = false,
  isRunning = false,
}: OracleFlowPanelProps) {
  let currentStep: FlowStep = 'connect';
  if (strategy.calls > 0) currentStep = 'recommend';
  else if (data.cost > 0) currentStep = 'write';
  else if (data.calls > 0) currentStep = 'scan';
  else if (chainChecked) currentStep = 'connect';

  const stepOrder: FlowStep[] = ['connect', 'scan', 'write', 'recommend'];
  const currentIdx = stepOrder.indexOf(currentStep);
  const hasAnyData = chainChecked || data.calls > 0 || strategy.calls > 0;

  const panelState = isRunning
    ? 'running'
    : hasAnyData ? 'data' : 'pending';

  const totalCost = data.cost + strategy.cost;
  const totalTokens = data.tokens + strategy.tokens;
  const totalCalls = data.calls + strategy.calls;

  return (
    <section className="oracle-panel" aria-label="Oracle flow">
      <div className="oracle-panel-header">
        <span className="oracle-panel-title">Oracle Flow</span>
        <span className={`oracle-panel-live${hasAnyData ? ' connected' : ''}`}>
          {panelState === 'pending' ? 'ready' : 'live'}
        </span>
      </div>

      <div className="oracle-panel-flow">
        {FLOW_STEPS.map((step, i) => {
          const status = hasAnyData
            ? i < currentIdx ? 'done' : i === currentIdx ? 'active' : ''
            : '';
          return (
            <div key={step.id}>
              {i > 0 && <div className="oracle-panel-step-connector" />}
              <div className={`oracle-panel-step${status ? ` oracle-panel-step--${status}` : ''}`}>
                <div className="oracle-panel-step-icon">
                  {status === 'done' ? '\u2713' : step.icon}
                </div>
                <div className="oracle-panel-step-body">
                  <div className="oracle-panel-step-label">{step.label}</div>
                  <div className="oracle-panel-step-desc">{step.desc}</div>
                </div>
              </div>
            </div>
          );
        })}
      </div>

      <div className="oracle-panel-metrics">
        <div className="oracle-panel-metric">
          <div className="oracle-panel-metric-label">Total Cost</div>
          <div className={`oracle-panel-metric-value${totalCost <= 0 ? ' oracle-panel-metric-value--empty' : ''}`}>
            {fmtCost(totalCost)}
          </div>
        </div>
        <div className="oracle-panel-metric">
          <div className="oracle-panel-metric-label">Tokens</div>
          <div className={`oracle-panel-metric-value${totalTokens <= 0 ? ' oracle-panel-metric-value--empty' : ''}`}>
            {fmtTokens(totalTokens)}
          </div>
        </div>
        <div className="oracle-panel-metric">
          <div className="oracle-panel-metric-label">Data Calls</div>
          <div className={`oracle-panel-metric-value${data.calls <= 0 ? ' oracle-panel-metric-value--empty' : ''}`}>
            {data.calls > 0 ? String(data.calls) : '--'}
          </div>
        </div>
        <div className="oracle-panel-metric">
          <div className="oracle-panel-metric-label">Total Calls</div>
          <div className={`oracle-panel-metric-value${totalCalls <= 0 ? ' oracle-panel-metric-value--empty' : ''}`}>
            {totalCalls > 0 ? String(totalCalls) : '--'}
          </div>
        </div>
      </div>
    </section>
  );
}
