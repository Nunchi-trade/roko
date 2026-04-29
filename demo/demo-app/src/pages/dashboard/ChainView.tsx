import { useState, useEffect, useRef, useCallback } from 'react';
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
  { label: 'SHA-256 hash per episode', desc: 'Cryptographic fingerprint for every agent turn' },
  { label: 'Merkle tree for batch verification', desc: 'Tree hashing enables O(log n) proof verification' },
  { label: 'EVM-compatible witness contract', desc: 'Ready for on-chain anchoring when chain backend connects' },
  { label: 'Automatic custody chain', desc: 'Gate results auto-append to tamper-evident audit log' },
  { label: 'HDC fingerprint embedding', desc: 'Hyperdimensional vectors encode episode semantics' },
  { label: 'Cross-agent verification', desc: 'Multi-party witness protocol for contested outcomes' },
];

/* ── Hash chain visualization ────────────────────────────── */

function HashChainViz({ episodes, height = 160 }: { episodes: number; height?: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const frameRef = useRef(0);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    if (rect.width === 0 || rect.height === 0) return;
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    ctx.clearRect(0, 0, w, h);

    // Background gradient
    const bg = ctx.createLinearGradient(0, 0, w, 0);
    bg.addColorStop(0, 'rgba(154,138,184,0.04)');
    bg.addColorStop(0.5, 'rgba(220,165,189,0.03)');
    bg.addColorStop(1, 'rgba(138,156,134,0.04)');
    ctx.fillStyle = bg;
    ctx.fillRect(0, 0, w, h);

    // Draw chain of blocks
    const blockCount = Math.min(12, Math.max(6, Math.floor(w / 90)));
    const blockW = 56;
    const blockH = 28;
    const gap = (w - blockCount * blockW) / (blockCount + 1);
    const cy = h / 2;

    for (let i = 0; i < blockCount; i++) {
      const x = gap + i * (blockW + gap);
      const y = cy - blockH / 2;

      // Connection line
      if (i > 0) {
        const prevX = gap + (i - 1) * (blockW + gap) + blockW;
        ctx.strokeStyle = 'rgba(220,165,189,0.2)';
        ctx.lineWidth = 1;
        ctx.setLineDash([3, 3]);
        ctx.beginPath();
        ctx.moveTo(prevX, cy);
        ctx.lineTo(x, cy);
        ctx.stroke();
        ctx.setLineDash([]);

        // Arrow
        ctx.fillStyle = 'rgba(220,165,189,0.3)';
        ctx.beginPath();
        ctx.moveTo(x - 4, cy - 3);
        ctx.lineTo(x, cy);
        ctx.lineTo(x - 4, cy + 3);
        ctx.fill();
      }

      // Block background
      const t = i / (blockCount - 1);
      const r = Math.round(154 + (138 - 154) * t);
      const g = Math.round(138 + (156 - 138) * t);
      const b = Math.round(184 + (134 - 184) * t);
      const blockColor = `rgb(${r},${g},${b})`;

      ctx.fillStyle = `rgba(${r},${g},${b},0.1)`;
      ctx.strokeStyle = `rgba(${r},${g},${b},0.3)`;
      ctx.lineWidth = 1;
      ctx.beginPath();
      ctx.roundRect(x, y, blockW, blockH, 4);
      ctx.fill();
      ctx.stroke();

      // Block hash snippet
      const hashStart = (i * 5) % (DEMO_HASH.length - 8);
      const snippet = DEMO_HASH.slice(hashStart, hashStart + 8);
      ctx.fillStyle = blockColor;
      ctx.font = '8px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText(snippet, x + blockW / 2, cy);

      // Block number
      ctx.fillStyle = 'rgba(255,255,255,0.25)';
      ctx.font = '7px "JetBrains Mono", monospace';
      ctx.fillText(`#${episodes - blockCount + i + 1}`, x + blockW / 2, y - 7);
    }

    // Subtle pulsing cursor at the end
    const lastX = gap + (blockCount - 1) * (blockW + gap) + blockW + 8;
    const pulse = Math.sin(frameRef.current * 0.05) * 0.3 + 0.5;
    ctx.fillStyle = `rgba(220,165,189,${pulse})`;
    ctx.fillRect(lastX, cy - 8, 2, 16);

    frameRef.current++;
    if (frameRef.current < 200) {
      requestAnimationFrame(draw);
    }
  }, [episodes]);

  useEffect(() => {
    frameRef.current = 0;
    draw();
  }, [draw]);

  return (
    <div style={{ position: 'relative', width: '100%', height, overflow: 'hidden' }}>
      <canvas ref={canvasRef} style={{ width: '100%', height: '100%', display: 'block' }} />
    </div>
  );
}

/* ── Component ───────────────────────────────────────────── */

export default function ChainView() {
  const { get } = useApiWithFallback();
  const [episodes, setEpisodes] = useState(847);
  const [gatesPassed, setGatesPassed] = useState(791);
  const [gatesFailed, setGatesFailed] = useState(56);
  const [gateHistory, setGateHistory] = useState<GateRun[]>([]);
  const [typedHash, setTypedHash] = useState('');
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  useEffect(() => {
    get<HealthResponse>('/api/health').then((h) => {
      const snap = h?.statehub?.snapshot;
      if (snap) {
        setEpisodes(snap.episodes_total ?? 847);
        setGatesPassed(snap.gates_passed ?? 791);
        setGatesFailed(snap.gates_failed ?? 56);
      }
    }).catch(() => {});

    get<GateRun[]>('/api/gates/history?limit=20&format=waterfall').then((data) => {
      if (Array.isArray(data)) setGateHistory(data);
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

  const gateTotal = gatesPassed + gatesFailed;
  const passRate = gateTotal > 0 ? (gatesPassed / gateTotal) * 100 : 93.4;

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12 }}>
      <style>{`
        @keyframes blink-cursor {
          0%, 100% { opacity: 1; }
          50% { opacity: 0; }
        }
      `}</style>

      {/* ═══ TOP MOSAIC ═══ */}
      <Mosaic columns={5}>
        <MosaicCell label="STATUS" value="Active" color="success" />
        <MosaicCell label="EPISODES" value={episodes.toLocaleString()} color="rose" mono sub="hashed" />
        <MosaicCell label="GATES PASSED" value={gatesPassed.toLocaleString()} color="success" mono />
        <MosaicCell label="GATES FAILED" value={gatesFailed.toLocaleString()} color="warning" mono />
        <MosaicCell label="PASS RATE" value={`${passRate.toFixed(1)}%`} color="bone" mono />
      </Mosaic>

      {/* ═══ HASH CHAIN VIZ ═══ */}
      <Pane
        title="HASH CHAIN"
        badge={
          <code style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--text-dim)', letterSpacing: '.02em' }}>
            {typedHash.slice(0, 24)}...
            <span style={{ display: 'inline-block', width: 1, height: '1em', background: 'var(--rose-dim)', marginLeft: 1, verticalAlign: 'text-bottom', animation: 'blink-cursor .8s step-end infinite' }} />
          </code>
        }
      >
        <HashChainViz episodes={episodes} height={120} />
      </Pane>

      {/* ═══ MIDDLE ROW: Features + Gate Waterfall ═══ */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 12 }}>
        <Pane title="CRYPTOGRAPHIC AGENT TRAIL">
          <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
            <p style={{
              fontFamily: 'var(--display)',
              fontSize: 12,
              fontWeight: 300,
              color: 'var(--text-soft)',
              lineHeight: 1.7,
              margin: '0 0 10px',
            }}>
              Every agent action is logged with cryptographic hashes. When the chain
              backend is connected, actions become tamper-proof witnesses anchored on-chain.
            </p>
            {FEATURES.map((f) => (
              <div key={f.label} style={{
                display: 'flex',
                alignItems: 'flex-start',
                gap: 8,
                padding: '6px 0',
                borderBottom: '1px solid rgba(255,255,255,.03)',
              }}>
                <span style={{ color: 'var(--success)', fontSize: 11, marginTop: 1, flexShrink: 0 }}>&#x2713;</span>
                <div>
                  <div style={{ fontFamily: 'var(--mono)', fontSize: 11, color: 'var(--text-primary)' }}>
                    {f.label}
                  </div>
                  <div style={{ fontFamily: 'var(--mono)', fontSize: 9, color: 'var(--text-ghost)', letterSpacing: '.02em', marginTop: 2 }}>
                    {f.desc}
                  </div>
                </div>
              </div>
            ))}
          </div>
        </Pane>

        <Pane title="GATE PIPELINE WATERFALL" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>7-rung</span>}>
          <GateWaterfall runs={gateHistory} height={280} />
        </Pane>
      </div>
    </div>
  );
}
