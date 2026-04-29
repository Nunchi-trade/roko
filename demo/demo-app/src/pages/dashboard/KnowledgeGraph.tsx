import { useState, useEffect, useRef, useCallback } from 'react';
import { useApiWithFallback } from '../../hooks/useApiWithFallback';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';

/* ── Types ───────────────────────────────────────────────── */

interface KnowledgeEntry {
  id: string;
  domain?: string;
  citations?: number;
  label?: string;
}

interface KnowledgeEdge {
  source: string;
  target: string;
  frequency?: number;
}

/* ── Domain color map ────────────────────────────────────── */

const DOMAIN_COLORS: Record<string, string> = {
  gate:      '#cc90a8', // rose
  agent:     '#c8b890', // bone
  knowledge: '#9494b4', // dream
  plan:      '#7a8a78', // success
  config:    '#c89a68', // warning
};

function domainColor(domain?: string): string {
  return DOMAIN_COLORS[domain ?? ''] ?? '#706070';
}

/* ── Force graph simulation ──────────────────────────────── */

interface SimNode {
  id: string;
  x: number;
  y: number;
  vx: number;
  vy: number;
  domain: string;
  label: string;
  citations: number;
}

function buildSimulation(entries: KnowledgeEntry[], edges: KnowledgeEdge[], w: number, h: number) {
  const nodes: SimNode[] = entries.map((e, i) => ({
    id: e.id,
    x: w / 2 + (Math.cos(i * 2.39996) * w * 0.3),
    y: h / 2 + (Math.sin(i * 2.39996) * h * 0.3),
    vx: 0,
    vy: 0,
    domain: e.domain ?? 'unknown',
    label: e.label ?? e.id,
    citations: e.citations ?? 1,
  }));

  const nodeMap = new Map(nodes.map((n) => [n.id, n]));

  const links = edges
    .filter((e) => nodeMap.has(e.source) && nodeMap.has(e.target))
    .map((e) => ({
      source: nodeMap.get(e.source)!,
      target: nodeMap.get(e.target)!,
      freq: e.frequency ?? 1,
    }));

  return { nodes, links };
}

function tick(nodes: SimNode[], links: { source: SimNode; target: SimNode; freq: number }[], w: number, h: number) {
  const cx = w / 2;
  const cy = h / 2;

  // Repulsion between nodes
  for (let i = 0; i < nodes.length; i++) {
    for (let j = i + 1; j < nodes.length; j++) {
      const a = nodes[i], b = nodes[j];
      let dx = b.x - a.x;
      let dy = b.y - a.y;
      const dist = Math.sqrt(dx * dx + dy * dy) || 1;
      const force = 800 / (dist * dist);
      dx = (dx / dist) * force;
      dy = (dy / dist) * force;
      a.vx -= dx;
      a.vy -= dy;
      b.vx += dx;
      b.vy += dy;
    }
  }

  // Spring attraction along edges
  for (const link of links) {
    const { source: a, target: b } = link;
    let dx = b.x - a.x;
    let dy = b.y - a.y;
    const dist = Math.sqrt(dx * dx + dy * dy) || 1;
    const force = (dist - 80) * 0.008;
    dx = (dx / dist) * force;
    dy = (dy / dist) * force;
    a.vx += dx;
    a.vy += dy;
    b.vx -= dx;
    b.vy -= dy;
  }

  // Gravity toward center
  for (const n of nodes) {
    n.vx += (cx - n.x) * 0.002;
    n.vy += (cy - n.y) * 0.002;
  }

  // Apply velocity with damping
  for (const n of nodes) {
    n.vx *= 0.85;
    n.vy *= 0.85;
    n.x += n.vx;
    n.y += n.vy;
    // Keep in bounds
    n.x = Math.max(30, Math.min(w - 30, n.x));
    n.y = Math.max(30, Math.min(h - 30, n.y));
  }
}

/* ── Component ───────────────────────────────────────────── */

export default function KnowledgeGraph() {
  const { get } = useApiWithFallback();
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [edges, setEdges] = useState<KnowledgeEdge[]>([]);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const simRef = useRef<ReturnType<typeof buildSimulation> | null>(null);
  const rafRef = useRef<number>(0);
  const frameRef = useRef<number>(0);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      const [eData, edData] = await Promise.all([
        get<KnowledgeEntry[] | { items?: KnowledgeEntry[] }>('/api/knowledge/entries'),
        get<KnowledgeEdge[] | { items?: KnowledgeEdge[] }>('/api/knowledge/edges'),
      ]);
      if (cancelled) return;
      const e = Array.isArray(eData) ? eData : ((eData as { items?: KnowledgeEntry[] }).items ?? []);
      const ed = Array.isArray(edData) ? edData : ((edData as { items?: KnowledgeEdge[] }).items ?? []);
      setEntries(e);
      setEdges(ed);
    };
    poll();
    const id = setInterval(poll, 30_000);
    return () => { cancelled = true; clearInterval(id); };
  }, [get]);

  /* Unique domains */
  const domains = new Set(entries.map((e) => e.domain).filter(Boolean));

  /* Canvas rendering */
  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas || entries.length === 0) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    const w = rect.width;
    const h = rect.height;
    canvas.width = w * dpr;
    canvas.height = h * dpr;
    ctx.scale(dpr, dpr);

    // Initialize simulation once
    if (!simRef.current || simRef.current.nodes.length !== entries.length) {
      simRef.current = buildSimulation(entries, edges, w, h);
    }

    const { nodes, links } = simRef.current;

    // Tick physics
    tick(nodes, links, w, h);

    // Clear
    ctx.clearRect(0, 0, w, h);

    // Draw edges
    ctx.lineWidth = 1;
    for (const link of links) {
      const alpha = Math.min(0.15 + link.freq * 0.03, 0.4);
      ctx.strokeStyle = `rgba(180,160,180,${alpha})`;
      ctx.beginPath();
      ctx.moveTo(link.source.x, link.source.y);
      ctx.lineTo(link.target.x, link.target.y);
      ctx.stroke();
    }

    // Draw nodes
    for (const node of nodes) {
      const r = 8 + node.citations * 1.5;
      const color = domainColor(node.domain);

      // Glow effect
      ctx.save();
      ctx.shadowColor = color;
      ctx.shadowBlur = 14;
      ctx.beginPath();
      ctx.arc(node.x, node.y, r, 0, Math.PI * 2);
      ctx.fillStyle = color;
      ctx.fill();
      ctx.restore();

      // Label
      ctx.fillStyle = 'rgba(196,180,196,0.85)';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.fillText(node.label, node.x, node.y + r + 14);
    }

    // Check energy — stop animation when settled
    frameRef.current++;
    const energy = nodes.reduce((sum, n) => sum + n.vx * n.vx + n.vy * n.vy, 0);
    if (energy > 0.1 && frameRef.current < 300) {
      rafRef.current = requestAnimationFrame(draw);
    }
  }, [entries, edges]);

  useEffect(() => {
    if (entries.length > 0) {
      // Reset sim when data changes
      simRef.current = null;
      frameRef.current = 0;
      rafRef.current = requestAnimationFrame(draw);
    }
    return () => {
      if (rafRef.current) cancelAnimationFrame(rafRef.current);
    };
  }, [draw, entries]);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 16, maxWidth: 1200 }}>
      {/* ═══ TOP MOSAIC ═══ */}
      <Mosaic columns={3}>
        <MosaicCell label="NODES" value={entries.length || 18} color="rose" mono />
        <MosaicCell label="EDGES" value={edges.length || 28} color="bone" mono />
        <MosaicCell label="DOMAINS" value={domains.size || 5} color="dream" mono />
      </Mosaic>

      {/* ═══ GRAPH CANVAS ═══ */}
      <Pane title="KNOWLEDGE GRAPH" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>{entries.length} nodes</span>}>
        <div style={{ position: 'relative', height: 380 }}>
          <canvas
            ref={canvasRef}
            style={{ width: '100%', height: '100%', display: 'block' }}
          />
          {/* HUD overlays */}
          <div style={{
            position: 'absolute', top: 8, left: 12,
            fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--text-dim)',
            letterSpacing: '.08em',
          }}>
            {entries.length} NODES / {edges.length} EDGES
          </div>
          <div style={{
            position: 'absolute', top: 8, right: 12,
            display: 'flex', gap: 10,
          }}>
            {Object.entries(DOMAIN_COLORS).map(([d, c]) => (
              <span key={d} style={{
                display: 'flex', alignItems: 'center', gap: 4,
                fontFamily: 'var(--mono)', fontSize: 10, color: 'var(--text-dim)',
                letterSpacing: '.06em',
              }}>
                <span style={{
                  width: 5, height: 5, borderRadius: '50%',
                  background: c, display: 'inline-block',
                }} />
                {d}
              </span>
            ))}
          </div>
          <div style={{
            position: 'absolute', bottom: 8, left: 12,
            fontFamily: 'var(--mono)', fontSize: 8, color: 'var(--text-dim)',
            letterSpacing: '.06em',
          }}>
            FORCE-DIRECTED / 2D
          </div>
        </div>
      </Pane>

      {/* ═══ ENTRIES TABLE ═══ */}
      <Pane title="ENTRIES" flat>
        <div style={{ maxHeight: 320, overflow: 'auto' }}>
          <table style={{
            width: '100%',
            borderCollapse: 'collapse',
            fontFamily: 'var(--mono)',
            fontSize: 11,
          }}>
            <thead>
              <tr style={{ borderBottom: '1px solid rgba(255,255,255,.06)' }}>
                {['ID', 'DOMAIN', 'LABEL', 'CITATIONS'].map((h) => (
                  <th key={h} style={{
                    textAlign: 'left',
                    padding: '8px 12px',
                    fontWeight: 400,
                    letterSpacing: '.1em',
                    color: 'var(--text-dim)',
                    fontSize: 10,
                  }}>
                    {h}
                  </th>
                ))}
              </tr>
            </thead>
            <tbody>
              {entries.map((e) => (
                <tr
                  key={e.id}
                  style={{
                    borderBottom: '1px solid rgba(255,255,255,.03)',
                    cursor: 'pointer',
                    transition: 'background .15s',
                  }}
                  onMouseEnter={(ev) => { (ev.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,.03)'; }}
                  onMouseLeave={(ev) => { (ev.currentTarget as HTMLElement).style.background = 'transparent'; }}
                >
                  <td style={{ padding: '6px 12px', color: 'var(--text-dim)' }}>{e.id}</td>
                  <td style={{ padding: '6px 12px' }}>
                    <span style={{
                      display: 'inline-flex', alignItems: 'center', gap: 4,
                      color: domainColor(e.domain),
                    }}>
                      <span style={{
                        width: 5, height: 5, borderRadius: '50%',
                        background: domainColor(e.domain),
                        display: 'inline-block',
                      }} />
                      {e.domain ?? 'unknown'}
                    </span>
                  </td>
                  <td style={{ padding: '6px 12px', color: 'var(--text-primary)' }}>{e.label ?? '-'}</td>
                  <td style={{ padding: '6px 12px', color: 'var(--bone-bright)' }}>{e.citations ?? 0}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Pane>
    </div>
  );
}
