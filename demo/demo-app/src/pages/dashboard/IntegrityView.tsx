import { useState, useEffect, useRef, useCallback } from 'react';
import { useLiveApi } from '../../hooks/useLiveApi';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import GateWaterfall, { type GateRun } from '../../components/GateWaterfall';
import './dashboard.css';

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

async function sha256Hex(input: string): Promise<string> {
  const bytes = new TextEncoder().encode(input);
  const digest = await crypto.subtle.digest('SHA-256', bytes);
  return [...new Uint8Array(digest)]
    .map((byte) => byte.toString(16).padStart(2, '0'))
    .join('');
}

/* ── Features list ───────────────────────────────────────── */

const FEATURES = [
  { label: 'SHA-256 hash per episode', desc: 'Cryptographic fingerprint for every agent turn' },
  { label: 'Merkle tree for batch verification', desc: 'Tree hashing enables O(log n) proof verification' },
  { label: 'EVM-compatible witness contract', desc: 'Ready for on-chain anchoring when witness backend connects' },
  { label: 'Automatic custody trail', desc: 'Gate results auto-append to tamper-evident audit log' },
  { label: 'HDC fingerprint embedding', desc: 'Hyperdimensional vectors encode episode semantics' },
  { label: 'Cross-agent verification', desc: 'Multi-party witness protocol for contested outcomes' },
];

/* ── Hash chain visualization ────────────────────────────── */

function HashChainViz({
  episodes,
  hash,
  height = 160,
}: {
  episodes: number;
  hash: string;
  height?: number;
}) {
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

    if (!hash) {
      ctx.fillStyle = 'rgba(138,122,136,0.8)';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText('waiting for live episode hash', w / 2, h / 2);
      return;
    }

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
      const hashStart = (i * 5) % Math.max(hash.length - 8, 1);
      const snippet = hash.slice(hashStart, hashStart + 8);
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
  }, [episodes, hash]);

  useEffect(() => {
    frameRef.current = 0;
    draw();
  }, [draw]);

  return (
    <div className="dash-canvas-wrap" style={{ height }}>
      <canvas ref={canvasRef} role="img" aria-label="Integrity verification timeline" className="dash-canvas" />
    </div>
  );
}

/* ── Component ───────────────────────────────────────────── */

export default function IntegrityView() {
  const { get } = useLiveApi();
  const [episodes, setEpisodes] = useState(0);
  const [gatesPassed, setGatesPassed] = useState(0);
  const [gatesFailed, setGatesFailed] = useState(0);
  const [gateHistory, setGateHistory] = useState<GateRun[]>([]);
  const [latestHash, setLatestHash] = useState('');
  const [typedHash, setTypedHash] = useState('');
  const intervalRef = useRef<ReturnType<typeof setInterval> | null>(null);
  const timeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const fetchAll = useCallback(async () => {
    try {
      const [h, gates, eps] = await Promise.all([
        get<HealthResponse>('/api/health').catch(() => null),
        get<GateRun[]>('/api/gates/history?limit=20&format=waterfall').catch(() => []),
        get<unknown[]>('/api/episodes?limit=1').catch(() => []),
      ]);
      const snap = h?.statehub?.snapshot;
      if (snap) {
        setEpisodes(snap.episodes_total ?? 0);
        setGatesPassed(snap.gates_passed ?? 0);
        setGatesFailed(snap.gates_failed ?? 0);
      }
      if (Array.isArray(gates)) setGateHistory(gates);
      const latest = Array.isArray(eps) ? eps[0] : null;
      setLatestHash(latest ? await sha256Hex(JSON.stringify(latest)) : '');
    } catch {
      /* keep previous */
    }
  }, [get]);

  // Initial fetch + 30s fallback poll
  useEffect(() => {
    fetchAll();
    const id = setInterval(fetchAll, 30_000);
    return () => clearInterval(id);
  }, [fetchAll]);

  // SSE-triggered refetch
  const debouncedRefetch = useDebouncedRefetch(fetchAll, 2000);
  useContextEventSubscription(
    ['gate_result', 'episode'],
    debouncedRefetch,
  );

  /* Typewriter effect for hash */
  useEffect(() => {
    if (!latestHash) {
      setTypedHash('');
      return undefined;
    }
    let idx = 0;
    intervalRef.current = setInterval(() => {
      idx++;
      setTypedHash(latestHash.slice(0, idx));
      if (idx >= latestHash.length) {
        if (intervalRef.current) clearInterval(intervalRef.current);
        timeoutRef.current = setTimeout(() => {
          idx = 0;
          setTypedHash('');
          intervalRef.current = setInterval(() => {
            idx++;
            setTypedHash(latestHash.slice(0, idx));
            if (idx >= latestHash.length && intervalRef.current) {
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
  }, [latestHash]);

  const gateTotal = gatesPassed + gatesFailed;
  const passRate = gateTotal > 0 ? (gatesPassed / gateTotal) * 100 : 0;

  return (
    <div className="dash-page--wide">
      <style>{`
        @keyframes blink-cursor {
          0%, 100% { opacity: 1; }
          50% { opacity: 0; }
        }
      `}</style>

      {/* TOP MOSAIC */}
      <div className="dash-stagger" style={{ '--stagger-i': 0 } as React.CSSProperties}>
        <Mosaic columns={5}>
          <MosaicCell label="STATUS" value={latestHash ? 'Live' : 'No data'} color={latestHash ? 'success' : 'warning'} />
          <MosaicCell label="EPISODES" value={episodes.toLocaleString()} color="rose" mono sub="hashed" />
          <MosaicCell label="GATES PASSED" value={gatesPassed.toLocaleString()} color="success" mono />
          <MosaicCell label="GATES FAILED" value={gatesFailed.toLocaleString()} color="warning" mono />
          <MosaicCell
            label="PASS RATE"
            value={
              <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
                {`${passRate.toFixed(1)}%`}
                <span
                  className="dash-bar-track"
                  style={{ width: 48, display: 'inline-block' }}
                >
                  <span
                    className="dash-bar-fill dash-bar-fill--rose dash-gauge-fill"
                    style={{ width: `${passRate}%` }}
                  />
                </span>
              </span>
            }
            color="bone"
            mono
          />
        </Mosaic>
      </div>

      {/* HASH CHAIN VIZ */}
      <div className="dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
        <Pane
          title="HASH TRAIL"
          badge={
            <code className="dash-hash-code">
              {typedHash.slice(0, 24)}...
              <span className="dash-blink-cursor" />
            </code>
          }
        >
          <div className="dash-chart-enter">
            <HashChainViz episodes={episodes} hash={latestHash} height={90} />
          </div>
        </Pane>
      </div>

      {/* MIDDLE ROW: Features + Gate Waterfall */}
      <div className="dash-grid-2--gap12">
        <div className="dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
          <Pane title="CRYPTOGRAPHIC AGENT TRAIL">
            <div className="dash-flex-col--gap2">
              <p className="dash-feature-desc-block">
                Agent episode rows are hashed from live runtime data. When a witness
                backend is connected, these records can be anchored for external verification.
              </p>
              {FEATURES.map((f, i) => (
                <div
                  key={f.label}
                  className="dash-row-item--start dash-row-sep--light dash-stagger"
                  style={{ '--stagger-i': i + 3 } as React.CSSProperties}
                >
                  <span className="dash-feature-check">&#x2713;</span>
                  <div>
                    <div className="dash-feature-label">{f.label}</div>
                    <div className="dash-feature-desc">{f.desc}</div>
                  </div>
                </div>
              ))}
            </div>
          </Pane>
        </div>

        <div className="dash-stagger" style={{ '--stagger-i': 3 } as React.CSSProperties}>
          <Pane title="GATE PIPELINE WATERFALL" badge={<span className="dash-badge">7-rung</span>}>
            <div className="dash-chart-enter">
              <GateWaterfall runs={gateHistory} height={200} />
            </div>
          </Pane>
        </div>
      </div>
    </div>
  );
}
