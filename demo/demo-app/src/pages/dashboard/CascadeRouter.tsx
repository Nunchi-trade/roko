import { type CSSProperties, useEffect, useMemo, useState } from 'react';
import StatCard from '../../components/StatCard';
import { useApiWithFallback } from '../../hooks/useApiWithFallback';

interface ConfidenceStat {
  successes: number;
  trials: number;
}

interface CascadeState {
  model_slugs?: string[];
  role_table?: Record<string, string>;
  confidence_stats?: Record<string, ConfidenceStat>;
  total_observations?: number;
}

const pageStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 18,
  minHeight: '100%',
  padding: '28px 40px 60px',
};

const headerStyle: CSSProperties = {
  display: 'flex',
  alignItems: 'flex-end',
  justifyContent: 'space-between',
  gap: 16,
};

const titleStyle: CSSProperties = {
  color: 'var(--rose-bright)',
  fontFamily: 'var(--font-serif)',
  fontSize: '1.45rem',
  fontWeight: 400,
  letterSpacing: 0,
};

const subStyle: CSSProperties = {
  color: 'var(--text-dim)',
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: '0.68rem',
};

const statGridStyle: CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(auto-fit, minmax(150px, 1fr))',
  gap: 12,
};

const tableWrapStyle: CSSProperties = {
  overflow: 'hidden',
  border: '1px solid var(--glass-2-border)',
  borderRadius: 8,
  background: 'var(--glass-bg)',
};

const tableStyle: CSSProperties = {
  width: '100%',
  borderCollapse: 'collapse',
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: '0.72rem',
};

const thStyle: CSSProperties = {
  padding: '10px 12px',
  color: 'var(--text-dim)',
  borderBottom: '1px solid var(--glass-2-border)',
  background: 'var(--raised)',
  fontWeight: 600,
  textAlign: 'left',
};

const tdStyle: CSSProperties = {
  padding: '11px 12px',
  color: 'var(--text)',
  borderBottom: '1px solid var(--glass-border)',
  verticalAlign: 'top',
};

const emptyStyle: CSSProperties = {
  padding: 36,
  color: 'var(--text-ghost)',
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: '0.75rem',
  textAlign: 'center',
};

function percent(value: number) {
  return `${(value * 100).toFixed(0)}%`;
}

export default function CascadeRouter() {
  const { get } = useApiWithFallback();
  const [state, setState] = useState<CascadeState>({});
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let cancelled = false;

    const poll = async () => {
      try {
        const data = await get<CascadeState>('/api/learn/cascade-router');
        if (cancelled) return;
        setState(data ?? {});
        setError(null);
      } catch {
        if (!cancelled) setError('Unable to load cascade router state');
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    poll();
    const id = window.setInterval(poll, 10000);

    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, [get]);

  const rows = useMemo(
    () => Object.entries(state.confidence_stats ?? {}).sort(([a], [b]) => a.localeCompare(b)),
    [state.confidence_stats],
  );

  const stats = useMemo(() => {
    const totalTrials = rows.reduce((sum, [, s]) => sum + s.trials, 0);
    const totalSuccesses = rows.reduce((sum, [, s]) => sum + s.successes, 0);

    return {
      totalTrials,
      avgConfidence: totalTrials > 0 ? totalSuccesses / totalTrials : 0,
    };
  }, [rows]);

  return (
    <div style={pageStyle}>
      <div style={headerStyle}>
        <div>
          <h1 style={titleStyle}>Cascade Router</h1>
          <div style={subStyle}>polling every 10s</div>
        </div>
        {error && <div style={{ ...subStyle, color: 'var(--fail)' }}>{error}</div>}
      </div>

      <div style={statGridStyle}>
        <StatCard label="Models" value={rows.length} color="rose" />
        <StatCard label="Observations" value={state.total_observations ?? stats.totalTrials} color="bone" />
        <StatCard label="Avg Confidence" value={percent(stats.avgConfidence)} color="sage" />
      </div>

      <div style={tableWrapStyle}>
        {loading ? (
          <div style={emptyStyle}>Loading cascade router...</div>
        ) : rows.length === 0 ? (
          <div style={emptyStyle}>No model stats found</div>
        ) : (
          <table style={tableStyle}>
            <thead>
              <tr>
                <th style={thStyle}>Model</th>
                <th style={thStyle}>Confidence</th>
                <th style={thStyle}>Successes</th>
                <th style={thStyle}>Trials</th>
              </tr>
            </thead>
            <tbody>
              {rows.map(([model, stat]) => (
                <tr key={model}>
                  <td style={tdStyle}>{model}</td>
                  <td style={tdStyle}>{percent(stat.trials > 0 ? stat.successes / stat.trials : 0)}</td>
                  <td style={tdStyle}>{stat.successes}</td>
                  <td style={tdStyle}>{stat.trials}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>

      {/* ═══ ROLE TABLE ═══ */}
      {state.role_table && Object.keys(state.role_table).length > 0 && (
        <div style={tableWrapStyle}>
          <table style={tableStyle}>
            <thead>
              <tr>
                <th style={thStyle}>Role</th>
                <th style={thStyle}>Assigned Model</th>
              </tr>
            </thead>
            <tbody>
              {Object.entries(state.role_table).sort(([a], [b]) => a.localeCompare(b)).map(([role, model]) => (
                <tr key={role}>
                  <td style={tdStyle}>{role}</td>
                  <td style={tdStyle}>{model}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      )}

      <div style={subStyle}>total observations: {state.total_observations ?? '—'}</div>
    </div>
  );
}
