import { relativeTime } from '../../lib/format';
import FlatIcon, { inferIcon } from '../../components/FlatIcon';
import type { Episode } from './types';
import { kindColor } from './types';

interface ExplorerCardsProps {
  episodes: Episode[];
  expandedEp: string | null;
  onToggleExpand: (id: string) => void;
  maxCostInSet: number;
  initialLoading: boolean;
  refreshing: boolean;
}

export default function ExplorerCards({
  episodes,
  expandedEp,
  onToggleExpand,
  maxCostInSet,
  initialLoading,
  refreshing,
}: ExplorerCardsProps) {
  return (
    <div className={`expl-cards-section${refreshing ? ' expl-refreshing' : ''}`}>
      {initialLoading ? (
        <div className="expl-card-grid progressive-reveal">
          {Array.from({ length: 6 }, (_, i) => (
            <div key={i} className="skeleton-card skeleton" style={{ height: 160 }} />
          ))}
        </div>
      ) : episodes.length === 0 ? (
        <div className="expl-empty">No episodes recorded yet</div>
      ) : (
        <div className="expl-card-grid">
          {episodes.slice(0, 30).map((ep, i) => (
            <div
              key={ep.id}
              className={`expl-card${expandedEp === ep.id ? ' expl-card--expanded' : ''}`}
              style={{ animationDelay: `${i * 60}ms` }}
              role="button"
              tabIndex={0}
              onClick={() => onToggleExpand(ep.id)}
              onKeyDown={(e) => {
                if (e.key === 'Enter' || e.key === ' ') {
                  e.preventDefault();
                  onToggleExpand(ep.id);
                }
              }}
            >
              {/* Top row: badges + time */}
              <div className="expl-card-top">
                <span className="expl-card-kind" style={{ background: kindColor(ep.kind) + '22', color: kindColor(ep.kind) }}>
                  <FlatIcon name={inferIcon(ep.kind)} size={13} tone="muted" />
                  {ep.kind}
                </span>
                {ep.model && (
                  <span className="expl-card-model"><FlatIcon name="model" size={13} tone="dream" />{ep.model}</span>
                )}
                <span className="expl-card-time">
                  <FlatIcon name="clock" size={13} tone="muted" />
                  {ep.timestamp_ms ? relativeTime(ep.timestamp_ms) : ''}
                </span>
              </div>

              {/* Agent name */}
              <div className="expl-card-agent"><FlatIcon name="agent" size={14} tone="rose" />{ep.agent_id ?? 'system'}</div>

              {/* Task ID */}
              {ep.task_id && (
                <div className="expl-card-task"><FlatIcon name="task" size={14} tone="bone" />{ep.task_id}</div>
              )}

              {/* Gate verdict dots */}
              {ep.gate_verdicts && ep.gate_verdicts.length > 0 && (
                <div className="expl-card-gates">
                  {ep.gate_verdicts.map((v, gi) => (
                    <span
                      key={gi}
                      className={`expl-gate-dot ${v.passed ? 'pass' : 'fail'}`}
                      title={`${v.gate}: ${v.passed ? 'passed' : 'failed'}`}
                    />
                  ))}
                </div>
              )}

              {/* Meta chips */}
              <div className="expl-card-meta">
                {ep.usage?.cost_usd != null && (
                  <span className="expl-chip cost"><FlatIcon name="cost" size={12} tone="bone" />${ep.usage.cost_usd.toFixed(3)}</span>
                )}
                {ep.duration_secs != null && (
                  <span className="expl-chip dur"><FlatIcon name="duration" size={12} tone="muted" />{ep.duration_secs.toFixed(1)}s</span>
                )}
                {ep.turns != null && (
                  <span className="expl-chip turns"><FlatIcon name="route" size={12} tone="dream" />{ep.turns}t</span>
                )}
              </div>

              {/* Cost bar */}
              <div className="expl-card-bar-wrap">
                <div
                  className="expl-card-bar"
                  style={{ width: `${Math.max(((ep.usage?.cost_usd ?? 0) / maxCostInSet) * 100, 2)}%` }}
                />
              </div>

              {/* Expanded detail */}
              {expandedEp === ep.id && (
                <div className="expl-card-detail">
                  {Object.entries(ep).map(([k, v]) => (
                    <div key={k} className="expl-card-field">
                      <span className="expl-card-field-key">{k}</span>
                      <span className="expl-card-field-val">
                        {typeof v === 'object' ? JSON.stringify(v) : String(v ?? '')}
                      </span>
                    </div>
                  ))}
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  );
}
