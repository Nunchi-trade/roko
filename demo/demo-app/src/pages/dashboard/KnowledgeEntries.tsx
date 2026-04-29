import { type CSSProperties, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import { useLiveApi } from '../../hooks/useLiveApi';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';

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
  gap: 10,
  minHeight: '100%',
};

const DOMAIN_COLORS: Record<string, string> = {
  gate: '#CC90A8',
  agent: '#C8B890',
  knowledge: '#9494B4',
  plan: '#7A8A78',
  config: '#C89A68',
};

function domainColor(domain?: string): string {
  return DOMAIN_COLORS[domain ?? ''] ?? '#706070';
}

function percent(value?: number) {
  if (typeof value !== 'number' || Number.isNaN(value)) return '—';
  return `${(value * 100).toFixed(0)}%`;
}

/* ── Domain distribution canvas ─────────────────────────── */

function DomainChart({ entries, height = 140 }: { entries: KnowledgeEntry[]; height?: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

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

    // Count domains
    const counts: Record<string, number> = {};
    for (const e of entries) {
      const d = e.domain ?? 'unknown';
      counts[d] = (counts[d] ?? 0) + 1;
    }
    const sorted = Object.entries(counts).sort(([, a], [, b]) => b - a);
    if (sorted.length === 0) {
      ctx.fillStyle = 'rgba(194,184,201,0.5)';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.fillText('No domain data', w / 2, h / 2);
      return;
    }

    const total = entries.length;
    const pad = { left: 80, right: 60, top: 8, bottom: 8 };
    const plotW = w - pad.left - pad.right;
    const barH = Math.min(22, (h - pad.top - pad.bottom) / sorted.length - 4);

    sorted.forEach(([domain, count], i) => {
      const y = pad.top + i * (barH + 4);
      const pct = count / total;
      const barW = pct * plotW;
      const color = domainColor(domain);

      // Label
      ctx.fillStyle = '#8a7a88';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'right';
      ctx.textBaseline = 'middle';
      ctx.fillText(domain, pad.left - 10, y + barH / 2);

      // Bar track
      ctx.fillStyle = 'rgba(255,255,255,0.03)';
      ctx.beginPath();
      ctx.roundRect(pad.left, y, plotW, barH, 3);
      ctx.fill();

      // Bar fill
      ctx.fillStyle = color;
      ctx.globalAlpha = 0.7;
      ctx.beginPath();
      ctx.roundRect(pad.left, y, Math.max(barW, 3), barH, 3);
      ctx.fill();
      ctx.globalAlpha = 1;

      // Value
      ctx.fillStyle = '#c4b4c4';
      ctx.font = '9px "JetBrains Mono", monospace';
      ctx.textAlign = 'left';
      ctx.fillText(`${count} (${(pct * 100).toFixed(0)}%)`, pad.left + barW + 8, y + barH / 2);
    });
  }, [entries]);

  useEffect(() => {
    draw();
    const ro = new ResizeObserver(draw);
    if (canvasRef.current) ro.observe(canvasRef.current);
    return () => ro.disconnect();
  }, [draw]);

  return (
    <div style={{ position: 'relative', width: '100%', height, overflow: 'hidden' }}>
      <canvas ref={canvasRef} style={{ width: '100%', height: '100%', display: 'block' }} />
    </div>
  );
}

/* ── Confidence distribution canvas ─────────────────────── */

function ConfidenceHistogram({ entries, height = 140 }: { entries: KnowledgeEntry[]; height?: number }) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

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

    const values = entries
      .map((e) => e.confidence)
      .filter((v): v is number => typeof v === 'number' && !Number.isNaN(v));

    if (values.length === 0) {
      ctx.fillStyle = 'rgba(194,184,201,0.5)';
      ctx.font = '10px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.fillText('No confidence data', w / 2, h / 2);
      return;
    }

    // 10 buckets: 0-10%, 10-20%, ...90-100%
    const buckets = new Array(10).fill(0);
    for (const v of values) {
      const idx = Math.min(Math.floor(v * 10), 9);
      buckets[idx]++;
    }
    const maxBucket = Math.max(...buckets, 1);

    const pad = { left: 36, right: 16, top: 8, bottom: 24 };
    const plotW = w - pad.left - pad.right;
    const plotH = h - pad.top - pad.bottom;
    const barW = plotW / 10 - 3;

    // Grid
    ctx.strokeStyle = 'rgba(255,255,255,0.04)';
    ctx.lineWidth = 1;
    for (let i = 0; i <= 3; i++) {
      const y = pad.top + plotH * (1 - i / 3);
      ctx.beginPath();
      ctx.moveTo(pad.left, y);
      ctx.lineTo(pad.left + plotW, y);
      ctx.stroke();
    }

    buckets.forEach((count, i) => {
      const x = pad.left + i * (plotW / 10) + 1.5;
      const barH = (count / maxBucket) * plotH;
      const y = pad.top + plotH - barH;

      // Determine color based on bucket position
      const t = i / 9;
      const r = Math.round(204 + (138 - 204) * t);
      const g = Math.round(144 + (156 - 144) * t);
      const b = Math.round(168 + (134 - 168) * t);

      ctx.fillStyle = `rgba(${r},${g},${b},0.6)`;
      ctx.beginPath();
      ctx.roundRect(x, y, barW, barH, [2, 2, 0, 0]);
      ctx.fill();

      // X-axis labels
      ctx.fillStyle = '#6a5a68';
      ctx.font = '8px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.fillText(`${i * 10}`, x + barW / 2, h - pad.bottom + 12);
    });

    // Y-axis labels
    ctx.fillStyle = '#6a5a68';
    ctx.font = '8px "JetBrains Mono", monospace';
    ctx.textAlign = 'right';
    ctx.fillText('0', pad.left - 6, pad.top + plotH + 3);
    ctx.fillText(String(maxBucket), pad.left - 6, pad.top + 3);
  }, [entries]);

  useEffect(() => {
    draw();
    const ro = new ResizeObserver(draw);
    if (canvasRef.current) ro.observe(canvasRef.current);
    return () => ro.disconnect();
  }, [draw]);

  return (
    <div style={{ position: 'relative', width: '100%', height, overflow: 'hidden' }}>
      <canvas ref={canvasRef} style={{ width: '100%', height: '100%', display: 'block' }} />
    </div>
  );
}

/* ── Table styles ────────────────────────────────────────── */

const thStyle: CSSProperties = {
  padding: '6px 10px',
  color: 'var(--text-dim)',
  borderBottom: '1px solid var(--glass-2-border)',
  background: 'var(--raised)',
  fontWeight: 600,
  textAlign: 'left',
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: '0.6rem',
  letterSpacing: '.08em',
  textTransform: 'uppercase',
};

const tdStyle: CSSProperties = {
  padding: '5px 10px',
  color: 'var(--text)',
  borderBottom: '1px solid var(--glass-border)',
  verticalAlign: 'middle',
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: '0.72rem',
};

/* ── Component ───────────────────────────────────────────── */

export default function KnowledgeEntries() {
  const { get } = useLiveApi();
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [lastLoaded, setLastLoaded] = useState<string>('—');

  const fetchEntries = useCallback(async () => {
    try {
      const data = await get<KnowledgeEntry[]>('/api/knowledge/entries');
      setEntries(Array.isArray(data) ? data : ((data as { items?: KnowledgeEntry[] }).items ?? []));
      setLastLoaded(new Date().toLocaleTimeString());
    } catch {
      /* keep previous data */
    } finally {
      setLoading(false);
    }
  }, [get]);

  // Initial fetch + 60s fallback poll
  useEffect(() => {
    fetchEntries();
    const id = window.setInterval(fetchEntries, 60_000);
    return () => window.clearInterval(id);
  }, [fetchEntries]);

  // SSE-triggered refetch
  const debouncedRefetch = useDebouncedRefetch(fetchEntries, 2000);
  useContextEventSubscription(
    ['knowledge_updated', 'knowledge_created', 'knowledge_deleted'],
    debouncedRefetch,
  );

  const stats = useMemo(() => {
    const domains = new Set(entries.map((entry) => entry.domain).filter(Boolean));
    const citationTotal = entries.reduce((sum, entry) => sum + (entry.citations ?? 0), 0);
    const confidenceValues = entries
      .map((entry) => entry.confidence)
      .filter((value): value is number => typeof value === 'number' && !Number.isNaN(value));
    const confidenceTotal = confidenceValues.reduce((sum, value) => sum + value, 0);
    const highConfidence = confidenceValues.filter((v) => v >= 0.8).length;

    return {
      domains: domains.size,
      citationTotal,
      avgCitations: entries.length > 0 ? citationTotal / entries.length : 0,
      avgConfidence: confidenceValues.length > 0 ? confidenceTotal / confidenceValues.length : undefined,
      highConfidence,
    };
  }, [entries]);

  return (
    <div style={pageStyle}>
      {/* ═══ TOP MOSAIC ═══ */}
      <Mosaic columns={5}>
        <MosaicCell label="TOTAL ENTRIES" value={entries.length} color="rose" mono sub={loading ? 'loading' : `updated ${lastLoaded}`} />
        <MosaicCell label="DOMAINS" value={stats.domains} color="bone" mono />
        <MosaicCell label="TOTAL CITATIONS" value={stats.citationTotal} color="dream" mono />
        <MosaicCell label="AVG CONFIDENCE" value={percent(stats.avgConfidence)} color="success" mono />
        <MosaicCell label="HIGH CONFIDENCE" value={stats.highConfidence} color="warning" mono sub=">= 80%" />
      </Mosaic>

      {/* ═══ CHARTS ROW ═══ */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
        <Pane
          title="DOMAIN DISTRIBUTION"
          badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>{stats.domains} domains</span>}
        >
          <DomainChart entries={entries} height={110} />
        </Pane>

        <Pane
          title="CONFIDENCE DISTRIBUTION"
          badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>histogram</span>}
        >
          <ConfidenceHistogram entries={entries} height={110} />
        </Pane>
      </div>

      {/* ═══ ENTRIES TABLE ═══ */}
      <Pane
        title="ALL ENTRIES"
        badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>{entries.length} rows</span>}
        flat
      >
        <div style={{ maxHeight: 280, overflow: 'auto' }}>
          {loading ? (
            <div style={{
              padding: 36,
              color: 'var(--text-ghost)',
              fontFamily: 'var(--mono)',
              fontSize: '0.75rem',
              textAlign: 'center',
            }}>
              Loading knowledge entries...
            </div>
          ) : entries.length === 0 ? (
            <div style={{
              padding: 36,
              color: 'var(--text-ghost)',
              fontFamily: 'var(--mono)',
              fontSize: '0.75rem',
              textAlign: 'center',
            }}>
              No knowledge entries found
            </div>
          ) : (
            <table style={{
              width: '100%',
              borderCollapse: 'collapse',
            }}>
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
                  <tr
                    key={entry.id}
                    style={{ transition: 'background .15s' }}
                    onMouseEnter={(ev) => { (ev.currentTarget as HTMLElement).style.background = 'rgba(255,255,255,.03)'; }}
                    onMouseLeave={(ev) => { (ev.currentTarget as HTMLElement).style.background = 'transparent'; }}
                  >
                    <td style={tdStyle}>{entry.label ?? entry.id}</td>
                    <td style={tdStyle}>
                      <span style={{ display: 'inline-flex', alignItems: 'center', gap: 6 }}>
                        <span style={{
                          width: 5, height: 5, borderRadius: '50%',
                          background: domainColor(entry.domain),
                          display: 'inline-block',
                          boxShadow: `0 0 6px ${domainColor(entry.domain)}80`,
                        }} />
                        <span style={{ color: domainColor(entry.domain) }}>{entry.domain ?? '—'}</span>
                      </span>
                    </td>
                    <td style={tdStyle}>{entry.citations ?? 0}</td>
                    <td style={tdStyle}>
                      <span style={{ display: 'inline-flex', alignItems: 'center', gap: 8 }}>
                        <span style={{
                          display: 'inline-block',
                          width: 48,
                          height: 4,
                          background: 'rgba(255,255,255,.04)',
                          borderRadius: 2,
                          overflow: 'hidden',
                        }}>
                          <span style={{
                            display: 'block',
                            height: '100%',
                            width: `${((entry.confidence ?? 0) * 100)}%`,
                            background: (entry.confidence ?? 0) >= 0.8 ? 'var(--success)' : (entry.confidence ?? 0) >= 0.5 ? 'var(--warning)' : 'var(--rose-bright)',
                            borderRadius: 2,
                          }} />
                        </span>
                        {percent(entry.confidence)}
                      </span>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </Pane>
    </div>
  );
}
