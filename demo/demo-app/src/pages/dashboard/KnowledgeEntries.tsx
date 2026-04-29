import { type CSSProperties, useEffect, useMemo, useState } from 'react';
import StatCard from '../../components/StatCard';
import { useApiWithFallback } from '../../hooks/useApiWithFallback';

interface KnowledgeEntry {
  id: string;
  domain?: string;
  citations?: number;
  label?: string;
  confidence?: number;
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

function percent(value?: number) {
  if (typeof value !== 'number' || Number.isNaN(value)) return '—';
  return `${(value * 100).toFixed(0)}%`;
}

export default function KnowledgeEntries() {
  const { get } = useApiWithFallback();
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [lastLoaded, setLastLoaded] = useState<string>('—');

  useEffect(() => {
    let cancelled = false;

    const poll = async () => {
      try {
        const data = await get<KnowledgeEntry[]>('/api/knowledge/entries');
        if (cancelled) return;
        setEntries(Array.isArray(data) ? data : []);
        setError(null);
        setLastLoaded(new Date().toLocaleTimeString());
      } catch {
        if (!cancelled) setError('Unable to load knowledge entries');
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    poll();
    const id = window.setInterval(poll, 30000);

    return () => {
      cancelled = true;
      window.clearInterval(id);
    };
  }, []);

  const stats = useMemo(() => {
    const domains = new Set(entries.map((entry) => entry.domain).filter(Boolean));
    const citationTotal = entries.reduce((sum, entry) => sum + (entry.citations ?? 0), 0);
    const confidenceValues = entries
      .map((entry) => entry.confidence)
      .filter((value): value is number => typeof value === 'number' && !Number.isNaN(value));
    const confidenceTotal = confidenceValues.reduce((sum, value) => sum + value, 0);

    return {
      domains: domains.size,
      avgCitations: entries.length > 0 ? citationTotal / entries.length : 0,
      avgConfidence: confidenceValues.length > 0 ? confidenceTotal / confidenceValues.length : undefined,
    };
  }, [entries]);

  return (
    <div style={pageStyle}>
      <div style={headerStyle}>
        <div>
          <h1 style={titleStyle}>Knowledge Entries</h1>
          <div style={subStyle}>{loading ? 'loading' : `updated ${lastLoaded}`}</div>
        </div>
        {error && <div style={{ ...subStyle, color: 'var(--fail)' }}>{error}</div>}
      </div>

      <div style={statGridStyle}>
        <StatCard label="Total" value={entries.length} color="rose" />
        <StatCard label="Domains" value={stats.domains} color="bone" />
        <StatCard label="Avg Citations" value={stats.avgCitations.toFixed(1)} color="sage" />
        <StatCard label="Avg Confidence" value={percent(stats.avgConfidence)} color="warn" />
      </div>

      <div style={tableWrapStyle}>
        {loading ? (
          <div style={emptyStyle}>Loading knowledge entries...</div>
        ) : entries.length === 0 ? (
          <div style={emptyStyle}>No knowledge entries found</div>
        ) : (
          <table style={tableStyle}>
            <thead>
              <tr>
                <th style={thStyle}>Label</th>
                <th style={thStyle}>Domain</th>
                <th style={thStyle}>Citations</th>
                <th style={thStyle}>Confidence</th>
              </tr>
            </thead>
            <tbody>
              {entries.map((entry) => (
                <tr key={entry.id}>
                  <td style={tdStyle}>{entry.label ?? entry.id}</td>
                  <td style={tdStyle}>{entry.domain ?? '—'}</td>
                  <td style={tdStyle}>{entry.citations ?? 0}</td>
                  <td style={tdStyle}>{percent(entry.confidence)}</td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  );
}
