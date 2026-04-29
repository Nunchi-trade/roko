import { useState, useEffect, useRef } from 'react';
import { useApiWithFallback } from '../../hooks/useApiWithFallback';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import GateWaterfall, { type GateRun } from '../../components/GateWaterfall';

/* ── Types ───────────────────────────────────────────────── */

interface HealthResponse {
  status?: string;
  uptime_secs?: number;
  version?: string;
  active_agents?: number;
  statehub?: {
    snapshot?: {
      episodes_total?: number;
      cost_usd_total?: number;
      gates_passed?: number;
      gates_failed?: number;
    };
  };
}

/* ── Fake SHA-256 hash for demo ──────────────────────────── */

const DEMO_HASH = 'a3f8c2d1e4b5976801234abcdef56789012345678abcdef0123456789abcdef0';

/* ── Features list ───────────────────────────────────────── */

const FEATURES = [
  'SHA-256 hash per episode',
  'Merkle tree for batch verification',
  'EVM-compatible witness contract',
  'Automatic custody chain on every gate result',
];

/* ── Component ───────────────────────────────────────────── */

export default function ChainView() {
  const { get } = useApiWithFallback();
  const [episodes, setEpisodes] = useState(847);
  const [gateResults, setGateResults] = useState(847);
  const [gateHistory, setGateHistory] = useState<GateRun[]>([]);
  const [typedHash, setTypedHash] = useState('');
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    get<HealthResponse>('/api/health').then((h) => {
      const snap = h?.statehub?.snapshot;
      if (snap) {
        setEpisodes(snap.episodes_total ?? 847);
        setGateResults((snap.gates_passed ?? 791) + (snap.gates_failed ?? 56));
      }
    }).catch(() => {});

    get<GateRun[]>('/api/gates/history?limit=20&format=waterfall').then((data) => {
      if (Array.isArray(data)) {
        setGateHistory(data);
      }
    }).catch(() => {});
  }, [get]);

  /* Typewriter effect for hash */
  useEffect(() => {
    let idx = 0;
    intervalRef.current = setInterval(() => {
      idx++;
      setTypedHash(DEMO_HASH.slice(0, idx));
      if (idx >= DEMO_HASH.length) {
        if (intervalRef.current) clearInterval(intervalRef.current);
        // Restart after a pause
        timeoutRef.current = setTimeout(() => {
          idx = 0;
          setTypedHash('');
          intervalRef.current = setInterval(() => {
            idx++;
            setTypedHash(DEMO_HASH.slice(0, idx));
            if (idx >= DEMO_HASH.length && intervalRef.current) {
              clearInterval(intervalRef.current);
            }
          }, 35);
        }, 3000);
      }
    }, 35);

    return () => {
      if (intervalRef.current) clearInterval(intervalRef.current);
      if (timeoutRef.current) clearTimeout(timeoutRef.current);
    };
  }, []);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, maxWidth: 1200 }}>
      <style>{`
        @keyframes blink-cursor {
          0%, 100% { opacity: 1; }
          50% { opacity: 0; }
        }
      `}</style>

      {/* ═══ TOP MOSAIC ═══ */}
      <Mosaic columns={4}>
        <MosaicCell label="STATUS" value="Active" color="success" />
        <MosaicCell label="EPISODES" value={episodes.toLocaleString()} color="rose" mono />
        <MosaicCell label="GATE RESULTS" value={gateResults.toLocaleString()} color="bone" mono />
        <MosaicCell label="HASH" value={
          <code style={{ fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--text-dim)', letterSpacing: '.02em' }}>
            {typedHash.slice(0, 16)}...
            <span style={{ display: 'inline-block', width: 1, height: '1em', background: 'var(--rose-dim)', marginLeft: 1, verticalAlign: 'text-bottom', animation: 'blink-cursor .8s step-end infinite' }} />
          </code>
        } color="dream" />
      </Mosaic>

      {/* ═══ COMBINED: explanation + features ═══ */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
        <Pane title="CRYPTOGRAPHIC AGENT TRAIL">
          <div style={{ padding: '4px 0 8px' }}>
            <p style={{ fontFamily: 'var(--display)', fontSize: 13, fontWeight: 300, color: 'var(--text-soft)', lineHeight: 1.7, margin: 0 }}>
              Every agent action is logged with cryptographic hashes. When the chain
              backend is connected, actions become tamper-proof witnesses anchored on-chain.
            </p>
            <div style={{ marginTop: 12, display: 'flex', flexDirection: 'column', gap: 4 }}>
              {FEATURES.map((f) => (
                <div key={f} style={{ display: 'flex', alignItems: 'center', gap: 8, fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--text-primary)' }}>
                  <span style={{ color: 'var(--success)', fontSize: 12 }}>&#x2713;</span>
                  <span>{f}</span>
                </div>
              ))}
            </div>
          </div>
        </Pane>

        <Pane title="GATE PIPELINE WATERFALL" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>7-rung</span>}>
          <GateWaterfall runs={gateHistory} height={260} />
        </Pane>
      </div>
    </div>
  );
}
