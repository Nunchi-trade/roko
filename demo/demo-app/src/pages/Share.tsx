import { useState, useEffect } from 'react';
import { useParams } from 'react-router';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import Pane from '../components/Pane';
import Mosaic, { MosaicCell } from '../components/Mosaic';
import GateBar from '../components/GateBar';

interface Receipt {
  prompt?: string;
  model?: string;
  cost_usd?: number;
  gate_results?: { name: string; status: 'pass' | 'fail' | 'skip' }[];
  created_at?: string;
  completed_at?: string;
}

export default function SharePage() {
  const { token } = useParams<{ token: string }>();
  const { get } = useApiWithFallback();
  const [receipt, setReceipt] = useState<Receipt | null>(null);
  const [error, setError] = useState(false);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    if (!token) return;
    (async () => {
      try {
        const r = await get<Receipt>(`/api/share/${token}`);
        setReceipt(r);
      } catch {
        setError(true);
      } finally {
        setLoaded(true);
      }
    })();
  }, [token, get]);

  if (!loaded) {
    return (
      <div style={{ padding: '120px 40px', textAlign: 'center' }}>
        <div style={{ fontFamily: 'var(--mono)', fontSize: 12, color: 'var(--text-dim)' }}>
          Loading receipt...
        </div>
      </div>
    );
  }

  if (error || !receipt) {
    return (
      <div style={{ padding: '120px 40px', maxWidth: 700, margin: '0 auto', textAlign: 'center' }}>
        <div style={{
          fontFamily: 'var(--display)',
          fontStyle: 'italic',
          fontSize: 24,
          color: 'var(--text-strong)',
          fontWeight: 300,
          marginBottom: 12,
        }}>
          Receipt not found
        </div>
        <div style={{
          fontFamily: 'var(--mono)',
          fontSize: 11,
          color: 'var(--text-dim)',
          letterSpacing: '.04em',
        }}>
          This share link may have expired or is invalid.
        </div>
      </div>
    );
  }

  const duration = receipt.created_at && receipt.completed_at
    ? `${Math.round((new Date(receipt.completed_at).getTime() - new Date(receipt.created_at).getTime()) / 1000)}s`
    : '4s';

  return (
    <div style={{ padding: '80px 40px', maxWidth: 700, margin: '0 auto' }}>
      <Pane
        title="EXECUTION RECEIPT"
        badge={<span>{receipt.model ?? 'claude-sonnet'}</span>}
        style={{ borderLeft: '3px solid var(--rose-dim)', marginTop: 0 }}
      >
        {receipt.prompt && (
          <div style={{ marginBottom: 24 }}>
            <div style={{
              fontFamily: 'var(--mono)',
              fontSize: 10,
              letterSpacing: '.18em',
              textTransform: 'uppercase' as const,
              color: 'var(--text-dim)',
              marginBottom: 8,
            }}>
              PROMPT
            </div>
            <div style={{
              fontFamily: 'var(--mono)',
              fontSize: 12,
              color: 'var(--text-primary)',
              lineHeight: 1.5,
            }}>
              {receipt.prompt}
            </div>
          </div>
        )}

        <Mosaic columns={3}>
          <MosaicCell label="MODEL" value={receipt.model ?? 'claude-sonnet'} color="rose" mono />
          <MosaicCell label="COST" value={receipt.cost_usd != null ? `$${receipt.cost_usd.toFixed(4)}` : '$0.024'} color="bone" mono />
          <MosaicCell label="DURATION" value={duration} color="success" mono />
        </Mosaic>

        {receipt.gate_results && (
          <div style={{ marginTop: 20 }}>
            <GateBar gates={receipt.gate_results.map((g) => ({
              name: g.name,
              status: g.status === 'skip' ? 'skip' : g.status === 'pass' ? 'pass' : 'fail',
            }))} />
          </div>
        )}
      </Pane>
    </div>
  );
}
