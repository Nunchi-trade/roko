import { useCallback, useEffect, useMemo, useRef, useState } from 'react';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import {
  AnimatedRow,
  AnimatedHeaderCell,
  TableEmptyState,
} from '../../components/AnimatedTable';
import { useLiveApi } from '../../hooks/useLiveApi';
import { getCssVar } from '../../lib/color';
import { domainColor } from '../../lib/palette';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import DataSurface from '../../components/design/DataSurface';
import '../../styles/table.css';
import './KnowledgeEntries.css';
import './dashboard.css';

interface KnowledgeEntry {
  id: string;
  domain?: string;
  citations?: number;
  label?: string;
  confidence?: number;
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
    const barH = Math.min(14, (h - pad.top - pad.bottom) / sorted.length - 4);

    sorted.forEach(([domain, count], i) => {
      const y = pad.top + i * (barH + 4);
      const pct = count / total;
      const barW = pct * plotW;
      const color = domainColor(domain);

      // Label
      ctx.fillStyle = getCssVar('--text-dim');
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
      ctx.globalAlpha = 0.5;
      ctx.beginPath();
      ctx.roundRect(pad.left, y, Math.max(barW, 3), barH, 2);
      ctx.fill();
      ctx.globalAlpha = 1;

      // Value
      ctx.fillStyle = getCssVar('--text-soft');
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
    <div className="dash-canvas-wrap" style={{ height }}>
      <canvas ref={canvasRef} role="img" aria-label="Knowledge entry frequency chart" className="dash-canvas" />
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
    const barW = plotW / 10 - 4;

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
      ctx.fillStyle = getCssVar('--text-ghost');
      ctx.font = '8px "JetBrains Mono", monospace';
      ctx.textAlign = 'center';
      ctx.fillText(`${i * 10}`, x + barW / 2, h - pad.bottom + 12);
    });

    // Y-axis labels
    ctx.fillStyle = getCssVar('--text-ghost');
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
    <div className="dash-canvas-wrap" style={{ height }}>
      <canvas ref={canvasRef} role="img" aria-label="Knowledge tier distribution chart" className="dash-canvas" />
    </div>
  );
}

/* ── Component ───────────────────────────────────────────── */

type KESortKey = 'label' | 'domain' | 'citations' | 'confidence';

export default function KnowledgeEntries() {
  const { get } = useLiveApi();
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [loading, setLoading] = useState(true);
  const [lastLoaded, setLastLoaded] = useState<string>('—');
  const [sortKey, setSortKey] = useState<KESortKey>('label');
  const [sortAsc, setSortAsc] = useState(true);

  function handleSort(key: string) {
    const k = key as KESortKey;
    if (sortKey === k) setSortAsc(!sortAsc);
    else { setSortKey(k); setSortAsc(true); }
  }

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
    ['knowledge_ingested', 'knowledge_consumed'],
    debouncedRefetch,
  );

  const sortedEntries = useMemo(() => {
    return [...entries].sort((a, b) => {
      let cmp = 0;
      switch (sortKey) {
        case 'label': cmp = (a.label ?? a.id).localeCompare(b.label ?? b.id); break;
        case 'domain': cmp = (a.domain ?? '').localeCompare(b.domain ?? ''); break;
        case 'citations': cmp = (a.citations ?? 0) - (b.citations ?? 0); break;
        case 'confidence': cmp = (a.confidence ?? 0) - (b.confidence ?? 0); break;
      }
      return sortAsc ? cmp : -cmp;
    });
  }, [entries, sortKey, sortAsc]);

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
    <DataSurface
      loading={loading}
      empty={!loading && entries.length === 0}
      emptyLabel="No knowledge entries found. Use roko knowledge to populate the store."
    >
    <div className="dash-page--full">
      {/* TOP MOSAIC */}
      <div className="dash-stagger" style={{ '--stagger-i': 0 } as React.CSSProperties}>
        <Mosaic columns={5}>
          <MosaicCell label="TOTAL ENTRIES" value={entries.length} color="rose" mono sub={loading ? 'loading' : `updated ${lastLoaded}`} />
          <MosaicCell label="DOMAINS" value={stats.domains} color="bone" mono />
          <MosaicCell label="TOTAL CITATIONS" value={stats.citationTotal} color="dream" mono />
          <MosaicCell label="AVG CONFIDENCE" value={percent(stats.avgConfidence)} color="success" mono />
          <MosaicCell label="HIGH CONFIDENCE" value={stats.highConfidence} color="warning" mono sub=">= 80%" />
        </Mosaic>
      </div>

      {/* CHARTS ROW */}
      <div className="dash-grid-2">
        <div className="dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
          <Pane
            title="DOMAIN DISTRIBUTION"
            badge={<span className="dash-badge">{stats.domains} domains</span>}
          >
            <div className="dash-chart-enter">
              <DomainChart entries={entries} height={110} />
            </div>
          </Pane>
        </div>

        <div className="dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
          <Pane
            title="CONFIDENCE DISTRIBUTION"
            badge={<span className="dash-badge">histogram</span>}
          >
            <div className="dash-chart-enter">
              <ConfidenceHistogram entries={entries} height={110} />
            </div>
          </Pane>
        </div>
      </div>

      {/* ENTRIES TABLE */}
      <div className="dash-stagger" style={{ '--stagger-i': 3 } as React.CSSProperties}>
        <Pane
          title="ALL ENTRIES"
          badge={<span className="dash-badge">{entries.length} rows</span>}
          flat
        >
          <div className="dash-table-scroll--280 dash-crossfade-enter">
            {loading ? (
              <div className="dash-placeholder--lg">Loading knowledge entries...</div>
            ) : (
              <table className="dash-table--plain">
                <thead>
                  <tr>
                    <AnimatedHeaderCell sortKey="label" currentSort={sortKey} ascending={sortAsc} onSort={handleSort} className="tbl-header">Label</AnimatedHeaderCell>
                    <AnimatedHeaderCell sortKey="domain" currentSort={sortKey} ascending={sortAsc} onSort={handleSort} className="tbl-header">Domain</AnimatedHeaderCell>
                    <AnimatedHeaderCell sortKey="citations" currentSort={sortKey} ascending={sortAsc} onSort={handleSort} className="tbl-header">Citations</AnimatedHeaderCell>
                    <AnimatedHeaderCell sortKey="confidence" currentSort={sortKey} ascending={sortAsc} onSort={handleSort} className="tbl-header">Confidence</AnimatedHeaderCell>
                  </tr>
                </thead>
                <tbody>
                  {sortedEntries.length === 0 ? (
                    <TableEmptyState colSpan={4} message="No knowledge entries found" />
                  ) : (
                    sortedEntries.map((entry, rowIdx) => (
                      <AnimatedRow
                        key={entry.id}
                        index={rowIdx}
                        tabIndex={0}
                        role="row"
                        onKeyDown={(e) => {
                          if (e.key === 'ArrowDown') { e.preventDefault(); (e.currentTarget.nextElementSibling as HTMLElement | null)?.focus(); }
                          if (e.key === 'ArrowUp') { e.preventDefault(); (e.currentTarget.previousElementSibling as HTMLElement | null)?.focus(); }
                        }}
                      >
                        <td className="tbl-cell">{entry.label ?? entry.id}</td>
                        <td className="tbl-cell">
                          <span className="dash-inline">
                            <span
                              className="dash-dot--5"
                              style={{
                                background: domainColor(entry.domain),
                                boxShadow: `0 0 6px ${domainColor(entry.domain)}80`,
                              }}
                            />
                            <span style={{ color: domainColor(entry.domain) }}>{entry.domain ?? '---'}</span>
                          </span>
                        </td>
                        <td className="tbl-cell">{entry.citations ?? 0}</td>
                        <td className="tbl-cell">
                          <span className="dash-inline--8">
                            <span className="dash-minibar">
                              <span
                                className="dash-minibar__fill dash-bar-animate"
                                style={{
                                  width: `${((entry.confidence ?? 0) * 100)}%`,
                                  background: (entry.confidence ?? 0) >= 0.8 ? 'var(--success)' : (entry.confidence ?? 0) >= 0.5 ? 'var(--warning)' : 'var(--rose-bright)',
                                  animationDelay: `${rowIdx * 30 + 200}ms`,
                                }}
                              />
                            </span>
                            {percent(entry.confidence)}
                          </span>
                        </td>
                      </AnimatedRow>
                    ))
                  )}
                </tbody>
              </table>
            )}
          </div>
        </Pane>
      </div>
    </div>
    </DataSurface>
  );
}
