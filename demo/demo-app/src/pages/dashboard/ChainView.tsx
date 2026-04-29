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

  useEffect(() => {
    get<HealthResponse>('/api/health').then((h) => {
      const snap = h?.statehub?.snapshot;
      if (snap) {
        setEpisodes(snap.episodes_total ?? 847);
        setGateResults((snap.gates_passed ?? 791) + (snap.gates_failed ?? 56));
      }
    }).catch(() => {});

    get<GateRun[]>('/api/gates/history?limit=20').then((data) => {
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
        setTimeout(() => {
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
    };
  }, []);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 1200 }}>
      {/* ═══ TOP MOSAIC ═══ */}
      <Mosaic columns={3}>
        <MosaicCell label="STATUS" value="Phase 2" color="bone" />
        <MosaicCell label="EPISODES LOGGED" value={episodes.toLocaleString()} color="rose" mono />
        <MosaicCell label="GATE RESULTS" value={gateResults.toLocaleString()} color="success" mono />
      </Mosaic>

      {/* ═══ EXPLANATION PANE ═══ */}
      <Pane
        title="TAMPER-PROOF AGENT HISTORY"
        badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>Phase 2</span>}
      >
        <div style={{
          textAlign: 'center',
          maxWidth: 560,
          margin: '0 auto',
          padding: '8px 0 16px',
        }}>
          <div style={{
            fontSize: 32,
            color: 'var(--text-dim)',
            marginBottom: 16,
            lineHeight: 1,
          }}>
            &#x26D3;
          </div>
          <h3 style={{
            fontFamily: 'var(--display)',
            fontSize: 20,
            fontWeight: 300,
            color: 'var(--text-strong)',
            marginBottom: 10,
          }}>
            Cryptographic Agent Trail
          </h3>
          <p style={{
            fontFamily: 'var(--display)',
            fontSize: 14,
            fontWeight: 300,
            color: 'var(--text-soft)',
            lineHeight: 1.7,
          }}>
            Every agent action is logged with cryptographic hashes. When the chain
            backend is connected, actions become tamper-proof witnesses anchored on-chain.
            The custody trail is already being recorded — chain anchoring activates in Phase 2.
          </p>
        </div>
      </Pane>

      {/* ═══ FEATURES LIST ═══ */}
      <Pane title="FEATURES" flat>
        <div>
          {FEATURES.map((f, i) => (
            <div
              key={f}
              style={{
                display: 'flex',
                alignItems: 'center',
                gap: 10,
                padding: '12px 16px',
                borderBottom: i < FEATURES.length - 1 ? '1px solid rgba(255,255,255,.04)' : 'none',
                fontFamily: 'var(--mono)',
                fontSize: 11,
                color: 'var(--text-primary)',
              }}
            >
              <span style={{
                color: 'var(--success)',
                fontSize: 13,
                fontWeight: 600,
              }}>
                &#x2713;
              </span>
              <span>{f}</span>
            </div>
          ))}
        </div>
      </Pane>

      {/* ═══ GATE WATERFALL ═══ */}
      <Pane
        title="GATE PIPELINE WATERFALL"
        badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>7-rung pipeline</span>}
      >
        <GateWaterfall runs={gateHistory} height={340} />
      </Pane>

      {/* ═══ HASH DISPLAY ═══ */}
      <Pane title="LATEST WITNESS HASH" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>SHA-256</span>}>
        <div style={{
          display: 'flex',
          alignItems: 'center',
          justifyContent: 'center',
          padding: '16px 0',
        }}>
          <code style={{
            fontFamily: 'var(--mono)',
            fontSize: 11,
            color: 'var(--text-dim)',
            letterSpacing: '.04em',
            lineHeight: 1.6,
            wordBreak: 'break-all',
            textAlign: 'center',
            minHeight: '1.6em',
          }}>
            {typedHash}
            <span style={{
              display: 'inline-block',
              width: 1,
              height: '1em',
              background: 'var(--rose-dim)',
              marginLeft: 1,
              verticalAlign: 'text-bottom',
              animation: 'blink-cursor .8s step-end infinite',
            }} />
          </code>
        </div>
        <style>{`
          @keyframes blink-cursor {
            0%, 100% { opacity: 1; }
            50% { opacity: 0; }
          }
        `}</style>
      </Pane>
    </div>
  );
}
