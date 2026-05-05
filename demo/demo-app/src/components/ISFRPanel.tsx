import { type InsightEvent } from './KnowledgeFlowPanel';
import './ISFRPanel.css';

interface ISFRPanelProps {
  insights: InsightEvent[];
  connected: boolean;
}

interface ClassRate {
  label: string;
  rate: string;
  weight: string;
  status: 'pending' | 'active' | 'done';
}

const DEFAULT_CLASSES: ClassRate[] = [
  { label: 'LENDING', rate: '--', weight: '60%', status: 'pending' },
  { label: 'STRUCTURED', rate: '--', weight: '25%', status: 'pending' },
  { label: 'FUNDING', rate: '--', weight: '10%', status: 'pending' },
  { label: 'STAKING', rate: '--', weight: '5%', status: 'pending' },
];

export default function ISFRPanel({ insights, connected }: ISFRPanelProps) {
  // Extract rates from insights content (best-effort parsing)
  const classes = DEFAULT_CLASSES.map(c => {
    const relevant = insights.filter(i =>
      i.content?.toUpperCase().includes(c.label)
    );
    if (relevant.length > 0) {
      // Try to extract a number from the most recent relevant insight
      const last = relevant[relevant.length - 1];
      const match = last.content?.match(/(\d+\.?\d*)\s*(?:bps|basis\s*points|%)/i);
      return {
        ...c,
        rate: match ? `${parseFloat(match[1]).toFixed(1)} bps` : c.rate,
        status: 'done' as const,
      };
    }
    return c;
  });

  // Try to find composite ISFR
  const isfrInsight = insights.find(i =>
    i.content?.toUpperCase().includes('ISFR') && i.content?.match(/\d+\.?\d*\s*(?:bps|basis)/i)
  );
  const isfrMatch = isfrInsight?.content?.match(/(\d+\.?\d*)\s*(?:bps|basis\s*points)/i);
  const compositeRate = isfrMatch ? parseFloat(isfrMatch[1]).toFixed(1) : '--';
  const compositePct = isfrMatch ? (parseFloat(isfrMatch[1]) / 100).toFixed(2) : '--';

  return (
    <div className="isfr-panel">
      <div className="isfr-panel-header">
        <span className="isfr-panel-title">ISFR</span>
        <span className={`isfr-panel-status ${connected ? 'connected' : ''}`}>
          {connected ? 'live' : 'offline'}
        </span>
      </div>

      <div className="isfr-composite">
        <div className="isfr-composite-rate">{compositeRate}</div>
        <div className="isfr-composite-unit">
          <span className="isfr-composite-bps">bps</span>
          <span className="isfr-composite-pct">{compositePct}%</span>
        </div>
        <div className="isfr-composite-label">Internet Secured Funding Rate</div>
      </div>

      <div className="isfr-classes">
        {classes.map(c => (
          <div key={c.label} className={`isfr-class isfr-class--${c.status}`}>
            <div className="isfr-class-header">
              <span className="isfr-class-label">{c.label}</span>
              <span className="isfr-class-weight">{c.weight}</span>
            </div>
            <div className="isfr-class-rate">{c.rate}</div>
            <div className="isfr-class-bar">
              <div
                className="isfr-class-bar-fill"
                style={{ width: c.status === 'done' ? '100%' : c.status === 'active' ? '50%' : '0%' }}
              />
            </div>
          </div>
        ))}
      </div>

      <div className="isfr-agents-summary">
        <div className="isfr-agents-title">Agent Activity</div>
        <div className="isfr-agents-count">
          {insights.length} insight{insights.length !== 1 ? 's' : ''} posted
        </div>
        {insights.slice(-5).reverse().map((ins, i) => (
          <div key={ins.id || i} className="isfr-insight-row">
            <span className="isfr-insight-type">{ins.type}</span>
            <span className="isfr-insight-content">
              {ins.content?.slice(0, 60)}{(ins.content?.length ?? 0) > 60 ? '...' : ''}
            </span>
          </div>
        ))}
      </div>
    </div>
  );
}
