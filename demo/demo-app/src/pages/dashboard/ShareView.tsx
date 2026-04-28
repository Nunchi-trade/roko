import { type CSSProperties, useEffect, useState } from 'react';
import { useParams } from 'react-router';
import GateBar from '../../components/GateBar';
import StatCard from '../../components/StatCard';
import { useApi } from '../../hooks/useApi';

interface RunTranscript {
  id: string;
  agent: string;
  role: string;
  prompt: string;
  success: boolean;
  gates: [string, boolean][];
  output?: string;
  cost_usd?: number;
  input_tokens?: number;
  output_tokens?: number;
  model?: string;
  duration_s?: number;
  episode_id?: string;
  timestamp: string;
}

const pageStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  gap: 18,
  minHeight: '100%',
  padding: '28px 40px 60px',
  overflow: 'auto',
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

const metaStyle: CSSProperties = {
  color: 'var(--text-dim)',
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: '0.68rem',
};

const paneStyle: CSSProperties = {
  border: '1px solid var(--glass-2-border)',
  borderRadius: 8,
  background: 'var(--glass-bg)',
};

const promptStyle: CSSProperties = {
  ...paneStyle,
  padding: 16,
  color: 'var(--text)',
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: '0.78rem',
  lineHeight: 1.6,
  whiteSpace: 'pre-wrap',
};

const statGridStyle: CSSProperties = {
  display: 'grid',
  gridTemplateColumns: 'repeat(4, 1fr)',
  gap: 16,
};

const outputStyle: CSSProperties = {
  ...paneStyle,
  padding: 16,
  fontSize: 12,
  color: 'var(--text-primary, var(--text))',
  fontFamily: 'var(--mono, var(--font-mono))',
  whiteSpace: 'pre-wrap',
  maxHeight: 400,
  overflow: 'auto',
};

const emptyStyle: CSSProperties = {
  ...paneStyle,
  padding: 36,
  color: 'var(--text-ghost)',
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: '0.75rem',
  textAlign: 'center',
};

const footerStyle: CSSProperties = {
  color: 'var(--text-dim)',
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: '0.68rem',
};

export default function ShareView() {
  const { token } = useParams<{ token: string }>();
  const { get } = useApi();
  const [report, setReport] = useState<RunTranscript | null>(null);
  const [error, setError] = useState(false);
  const [loading, setLoading] = useState(true);

  useEffect(() => {
    let cancelled = false;

    const load = async () => {
      if (!token) {
        setError(true);
        setLoading(false);
        return;
      }

      try {
        const data = await get<RunTranscript>(`/api/shared/${token}`);
        if (cancelled) return;
        setReport(data);
        setError(false);
      } catch {
        if (!cancelled) {
          setReport(null);
          setError(true);
        }
      } finally {
        if (!cancelled) setLoading(false);
      }
    };

    load();

    return () => {
      cancelled = true;
    };
  }, [get, token]);

  if (loading) {
    return (
      <div style={pageStyle}>
        <div style={emptyStyle}>Loading shared run...</div>
      </div>
    );
  }

  if (error || !report) {
    return (
      <div style={pageStyle}>
        <div style={emptyStyle}>Shared run not found</div>
      </div>
    );
  }

  const gateItems = report.gates.map(([name, passed]) => ({
    name,
    status: passed ? ('pass' as const) : ('fail' as const),
  }));

  return (
    <div style={pageStyle}>
      <div style={headerStyle}>
        <div>
          <h1 style={titleStyle}>{report.agent}</h1>
          <div style={metaStyle}>
            {report.role} · {report.timestamp}
          </div>
        </div>
        <div style={metaStyle}>{report.id}</div>
      </div>

      <div className="pane" style={promptStyle}>
        {report.prompt}
      </div>

      <div style={statGridStyle}>
        <StatCard label="Result" value={report.success ? 'PASS' : 'FAIL'} color={report.success ? 'sage' : 'warn'} />
        <StatCard label="Model" value={report.model ?? '—'} color="rose" />
        <StatCard
          label="Cost"
          value={report.cost_usd != null ? `$${report.cost_usd.toFixed(4)}` : '—'}
          color="bone"
        />
        <StatCard
          label="Duration"
          value={report.duration_s != null ? `${report.duration_s.toFixed(1)}s` : '—'}
          color="sage"
        />
      </div>

      <GateBar gates={gateItems} />

      {report.output && (
        <pre className="pane" style={outputStyle}>
          {report.output}
        </pre>
      )}

      <footer style={footerStyle}>Episode ID: {report.episode_id ?? '—'}</footer>
    </div>
  );
}
