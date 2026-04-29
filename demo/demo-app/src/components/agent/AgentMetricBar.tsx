import './AgentMetricBar.css';

interface AgentMetric {
  label: string;
  value: string | number;
  unit?: string;
  trend?: 'up' | 'down' | 'flat';
}

interface AgentMetricBarProps {
  metrics: AgentMetric[];
  compact?: boolean;
  className?: string;
}

const TREND_SYMBOLS: Record<string, string> = {
  up: '\u25B2',    // ▲
  down: '\u25BC',  // ▼
  flat: '\u2014',  // —
};

export default function AgentMetricBar({
  metrics,
  compact = false,
  className,
}: AgentMetricBarProps) {
  return (
    <div
      className={[
        'agent-metric-bar',
        compact && 'agent-metric-bar--compact',
        className,
      ].filter(Boolean).join(' ')}
    >
      {metrics.map((m) => (
        <div key={m.label} className="agent-metric-bar__item">
          <span className="agent-metric-bar__label">{m.label}</span>
          <span className="agent-metric-bar__value-row">
            <span className="agent-metric-bar__value">{m.value}</span>
            {m.unit && <span className="agent-metric-bar__unit">{m.unit}</span>}
            {m.trend && (
              <span className={`agent-metric-bar__trend agent-metric-bar__trend--${m.trend}`}>
                {TREND_SYMBOLS[m.trend]}
              </span>
            )}
          </span>
        </div>
      ))}
    </div>
  );
}
