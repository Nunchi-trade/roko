import { useState, useEffect, useRef, useCallback } from 'react';
import { useLiveApi } from '../../hooks/useLiveApi';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
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
  const { get } = useLiveApi();
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [edges, setEdges] = useState<KnowledgeEdge[]>([]);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const simRef = useRef<ReturnType<typeof buildSimulation> | null>(null);
  const rafRef = useRef<number>(0);
  const frameRef = useRef<number>(0);

  const fetchAll = useCallback(async () => {
    const [eData, edData] = await Promise.all([
      get<KnowledgeEntry[] | { items?: KnowledgeEntry[] }>('/api/knowledge/entries'),
      get<KnowledgeEdge[] | { items?: KnowledgeEdge[] }>('/api/knowledge/edges'),
    ]);
    const e = Array.isArray(eData) ? eData : ((eData as { items?: KnowledgeEntry[] }).items ?? []);
    const ed = Array.isArray(edData) ? edData : ((edData as { items?: KnowledgeEdge[] }).items ?? []);
    setEntries(e);
    setEdges(ed);
  }, [get]);

  // Initial fetch + 60s fallback poll
  useEffect(() => {
    fetchAll();
    const id = setInterval(fetchAll, 60_000);
    return () => clearInterval(id);
  }, [fetchAll]);

  // SSE-triggered refetch
  const debouncedRefetch = useDebouncedRefetch(fetchAll, 2000);
  useContextEventSubscription(
    ['knowledge_updated', 'knowledge_created', 'knowledge_deleted'],
    debouncedRefetch,
  );

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

  /* Citation stats */
  const totalCitations = entries.reduce((s, e) => s + (e.citations ?? 0), 0);
  const avgCitations = entries.length > 0 ? totalCitations / entries.length : 0;

  /* Domain counts */
  const domainCounts: Record<string, number> = {};
  for (const e of entries) {
    const d = e.domain ?? 'unknown';
    domainCounts[d] = (domainCounts[d] ?? 0) + 1;
  }
  const sortedDomains = Object.entries(domainCounts).sort(([, a], [, b]) => b - a);

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      {/* ═══ TOP MOSAIC ═══ */}
      <Mosaic columns={5}>
        <MosaicCell label="NODES" value={entries.length} color="rose" mono />
        <MosaicCell label="EDGES" value={edges.length} color="bone" mono />
        <MosaicCell label="DOMAINS" value={domains.size} color="dream" mono />
        <MosaicCell label="CITATIONS" value={totalCitations} color="warning" mono />
        <MosaicCell label="AVG CITATIONS" value={avgCitations.toFixed(1)} color="success" mono />
      </Mosaic>

      {/* ═══ GRAPH + DOMAIN BREAKDOWN ═══ */}
      <div style={{ display: 'grid', gridTemplateColumns: '2fr 1fr', gap: 10 }}>
        <Pane title="KNOWLEDGE GRAPH" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>force-directed</span>}>
          <div style={{ position: 'relative', height: 260 }}>
            <canvas
              ref={canvasRef}
              style={{ width: '100%', height: '100%', display: 'block' }}
            />
            {/* HUD overlays */}
            <div style={{
              position: 'absolute', top: 8, left: 12,
              fontFamily: 'var(--mono)', fontSize: 13, color: 'var(--text-dim)',
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
                  fontFamily: 'var(--mono)', fontSize: 13, color: 'var(--text-dim)',
                  letterSpacing: '.06em',
                }}>
                  <span style={{
                    width: 5, height: 5, borderRadius: '50%',
                    background: c, display: 'inline-block',
                    boxShadow: `0 0 4px ${c}80`,
                  }} />
                  {d}
                </span>
              ))}
            </div>
            <div style={{
              position: 'absolute', bottom: 8, left: 12,
              fontFamily: 'var(--mono)', fontSize: 15, color: 'var(--text-dim)',
              letterSpacing: '.06em',
            }}>
              FORCE-DIRECTED / 2D
            </div>
          </div>
        </Pane>

        {/* Domain Breakdown */}
        <Pane title="DOMAIN BREAKDOWN" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>{sortedDomains.length} domains</span>}>
          <div style={{ display: 'flex', flexDirection: 'column', gap: 0 }}>
            {sortedDomains.map(([domain, count], i) => {
              const total = entries.length || 1;
              const pct = (count / total) * 100;
              const color = domainColor(domain);

              return (
                <div
                  key={domain}
                  style={{
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 6,
                    padding: '6px 0',
                    borderBottom: i < sortedDomains.length - 1 ? '1px solid rgba(255,255,255,.04)' : 'none',
                  }}
                >
                  <div style={{ display: 'flex', alignItems: 'center', justifyContent: 'space-between', gap: 8 }}>
                    <span style={{ display: 'flex', alignItems: 'center', gap: 6 }}>
                      <span style={{
                        width: 6, height: 6, borderRadius: '50%',
                        background: color,
                        boxShadow: `0 0 6px ${color}60`,
                        display: 'inline-block',
                      }} />
                      <span style={{
                        fontFamily: 'var(--display)',
                        fontSize: 15,
                        fontWeight: 500,
                        color,
                        letterSpacing: '.02em',
                      }}>
                        {domain}
                      </span>
                    </span>
                    <span style={{
                      fontFamily: 'var(--mono)',
                      fontSize: '0.7rem',
                      fontWeight: 500,
                      color: 'var(--text-primary)',
                    }}>
                      {count}
                    </span>
                  </div>
                  <div style={{
                    height: 4,
                    background: 'rgba(255,255,255,.04)',
                    borderRadius: 2,
                    overflow: 'hidden',
                  }}>
                    <div style={{
                      height: '100%',
                      width: `${pct}%`,
                      background: color,
                      borderRadius: 2,
                      opacity: 0.7,
                      boxShadow: `0 0 8px ${color}40`,
                      transition: 'width .6s cubic-bezier(.22,1,.36,1)',
                    }} />
                  </div>
                  <span style={{
                    fontFamily: 'var(--mono)',
                    fontSize: '0.55rem',
                    color: 'var(--text-ghost)',
                  }}>
                    {pct.toFixed(0)}% of graph
                  </span>
                </div>
              );
            })}
          </div>
        </Pane>
      </div>

      {/* ═══ ENTRIES TABLE ═══ */}
      <Pane title="ALL ENTRIES" badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>{entries.length} nodes</span>} flat>
        <div style={{ maxHeight: 200, overflow: 'auto' }}>
          <table style={{
            width: '100%',
            borderCollapse: 'collapse',
            fontFamily: 'var(--mono)',
            fontSize: 14,
          }}>
            <thead>
              <tr style={{ borderBottom: '1px solid rgba(255,255,255,.06)' }}>
                {['ID', 'DOMAIN', 'LABEL', 'CITATIONS'].map((h) => (
                  <th key={h} style={{
                    textAlign: 'left',
                    padding: '8px 12px',
                    fontWeight: 600,
                    letterSpacing: '.1em',
                    color: 'var(--text-dim)',
                    fontSize: 13,
                    textTransform: 'uppercase',
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
                  <td style={{ padding: '8px 12px', color: 'var(--text-dim)' }}>{e.id}</td>
                  <td style={{ padding: '8px 12px' }}>
                    <span style={{
                      display: 'inline-flex', alignItems: 'center', gap: 6,
                      color: domainColor(e.domain),
                    }}>
                      <span style={{
                        width: 5, height: 5, borderRadius: '50%',
                        background: domainColor(e.domain),
                        display: 'inline-block',
                        boxShadow: `0 0 4px ${domainColor(e.domain)}60`,
                      }} />
                      {e.domain ?? 'unknown'}
                    </span>
                  </td>
                  <td style={{ padding: '8px 12px', color: 'var(--text-primary)' }}>{e.label ?? '-'}</td>
                  <td style={{ padding: '8px 12px', color: 'var(--bone-bright)' }}>{e.citations ?? 0}</td>
                </tr>
              ))}
            </tbody>
          </table>
        </div>
      </Pane>
    </div>
  );
}
