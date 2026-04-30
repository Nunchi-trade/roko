import { useState, useEffect, useCallback } from 'react';
import { useParams } from 'react-router';
import { useLiveApi } from '../hooks/useLiveApi';
import GateBar from '../components/GateBar';
import './Share.css';

interface Receipt {
  prompt?: string;
  model?: string;
  cost_usd?: number;
  tokens?: number;
  gate_results?: { name: string; status: 'pass' | 'fail' | 'skip' }[];
  created_at?: string;
  completed_at?: string;
}

/* ── Skeleton loading state ── */
function ShareSkeleton() {
  return (
    <div className="share-card-border share-skeleton">
      <div className="share-card-inner">
        {/* Head */}
        <div style={{ padding: 'var(--sp-1) var(--sp-3)', borderBottom: '1px solid var(--border)' }}>
          <div className="share-skel-line title" />
        </div>
        {/* Prompt */}
        <div>
          <div className="share-skel-line prompt" />
          <div className="share-skel-line prompt-2" />
        </div>
        {/* Stats */}
        <div className="share-skel-stats">
          {[0, 1, 2].map((i) => (
            <div key={i} className="share-skel-stat">
              <div className="share-skel-line" />
              <div className="share-skel-line" />
            </div>
          ))}
        </div>
        {/* Actions */}
        <div className="share-skel-actions">
          {[0, 1, 2].map((i) => (
            <div key={i} className="share-skel-line share-skel-btn" />
          ))}
        </div>
      </div>
    </div>
  );
}

/* ── Stat gauge ── */
function StatGauge({ label, value, fill }: { label: string; value: string; fill: number }) {
  return (
    <div className="share-stat">
      <span className="share-stat-label">{label}</span>
      <span className="share-stat-value">{value}</span>
      <div className="share-stat-bar">
        <div className="share-stat-fill" style={{ width: `${Math.min(fill, 100)}%` }} />
      </div>
    </div>
  );
}

/* ── Copy button with feedback ── */
function CopyButton({ text }: { text: string }) {
  const [copied, setCopied] = useState(false);

  const handleCopy = useCallback(async () => {
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      /* clipboard not available */
    }
    setCopied(true);
    setTimeout(() => setCopied(false), 2000);
  }, [text]);

  return (
    <div style={{ position: 'relative', display: 'inline-flex' }}>
      {copied && <span className="share-copy-tooltip">Copied!</span>}
      <button className={`share-copy-btn${copied ? ' copied' : ''}`} onClick={handleCopy}>
        {copied ? '\u2713' : '\u2398'} {copied ? 'COPIED' : 'COPY LINK'}
      </button>
    </div>
  );
}

export default function SharePage() {
  const { token } = useParams<{ token: string }>();
  const { get } = useLiveApi();
  const [receipt, setReceipt] = useState<Receipt | null>(null);
  const [error, setError] = useState(false);
  const [loaded, setLoaded] = useState(false);

  useEffect(() => {
    if (!token) {
      setLoaded(true);
      setError(true);
      return;
    }
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

  /* Loading skeleton */
  if (!loaded) {
    return (
      <div className="share-page">
        <ShareSkeleton />
      </div>
    );
  }

  /* Error state */
  if (error || !receipt) {
    return (
      <div className="share-error">
        <div className="share-error-title">Receipt not found</div>
        <div className="share-error-sub">
          This share link may have expired or is invalid.
        </div>
      </div>
    );
  }

  const duration =
    receipt.created_at && receipt.completed_at
      ? Math.round(
          (new Date(receipt.completed_at).getTime() - new Date(receipt.created_at).getTime()) /
            1000,
        )
      : 4;

  const costStr = receipt.cost_usd != null ? `$${receipt.cost_usd.toFixed(4)}` : '$0.024';
  const passCount = receipt.gate_results?.filter((g) => g.status === 'pass').length ?? 0;
  const totalGates = receipt.gate_results?.length ?? 0;
  const passRate = totalGates > 0 ? Math.round((passCount / totalGates) * 100) : 100;
  const shareUrl = typeof window !== 'undefined' ? window.location.href : '';

  return (
    <div className="share-page">
      <div className="share-card-border">
        <div className="share-card-inner share-stagger">
          {/* Head */}
          <div
            style={{
              display: 'flex',
              justifyContent: 'space-between',
              alignItems: 'center',
              padding: 'var(--sp-1) var(--sp-3)',
              borderBottom: '1px solid var(--border)',
              fontFamily: 'var(--mono)',
              fontSize: 'var(--text-xs)',
              letterSpacing: '.10em',
              textTransform: 'uppercase',
              color: 'var(--text-dim)',
            }}
          >
            <span style={{ color: 'var(--text-strong)', fontWeight: 500 }}>
              EXECUTION RECEIPT
            </span>
            <span style={{ color: 'var(--rose-glow)' }}>
              {receipt.model ?? 'claude-sonnet'}
            </span>
          </div>

          {/* Prompt */}
          {receipt.prompt && (
            <div className="share-prompt-section">
              <div className="share-prompt-label">PROMPT</div>
              <div className="share-prompt-text">{receipt.prompt}</div>
            </div>
          )}

          {/* Stat gauges */}
          <div className="share-stat-row">
            <StatGauge
              label="MODEL"
              value={receipt.model ?? 'claude-sonnet'}
              fill={100}
            />
            <StatGauge
              label="COST"
              value={costStr}
              fill={Math.min((receipt.cost_usd ?? 0.024) * 1000, 100)}
            />
            <StatGauge
              label="DURATION"
              value={`${duration}s`}
              fill={Math.min(duration * 5, 100)}
            />
          </div>

          {/* Gate results */}
          {receipt.gate_results && (
            <div className="share-gates">
              <div style={{ marginBottom: 'var(--sp-2)' }}>
                <span
                  style={{
                    fontFamily: 'var(--mono)',
                    fontSize: 'var(--text-xs)',
                    letterSpacing: '.12em',
                    textTransform: 'uppercase',
                    color: 'var(--text-dim)',
                  }}
                >
                  GATES{' '}
                  <span style={{ color: passRate === 100 ? 'var(--success)' : 'var(--rose-bright)' }}>
                    {passRate}%
                  </span>
                </span>
              </div>
              <GateBar
                gates={receipt.gate_results.map((g) => ({
                  name: g.name,
                  status: g.status === 'skip' ? 'skip' : g.status === 'pass' ? 'pass' : 'fail',
                }))}
              />
            </div>
          )}

          {/* Actions row */}
          <div className="share-actions">
            <CopyButton text={shareUrl} />
            <div style={{ marginLeft: 'auto', display: 'flex', gap: 'var(--sp-2)' }}>
              <button
                className="share-social-btn twitter"
                title="Share on X"
                onClick={() =>
                  window.open(
                    `https://x.com/intent/tweet?text=${encodeURIComponent(`Roko execution: ${receipt.model ?? 'claude-sonnet'} — ${costStr} in ${duration}s`)}&url=${encodeURIComponent(shareUrl)}`,
                    '_blank',
                  )
                }
              >
                X
              </button>
              <button
                className="share-social-btn github"
                title="Share on GitHub"
                onClick={() =>
                  window.open(
                    `https://github.com/nunchi/roko`,
                    '_blank',
                  )
                }
              >
                GH
              </button>
              <button
                className="share-social-btn link"
                title="Copy link"
                onClick={() => {
                  void navigator.clipboard.writeText(shareUrl);
                }}
              >
                #
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
