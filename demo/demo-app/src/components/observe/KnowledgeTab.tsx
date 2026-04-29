import { useState, useRef, useEffect, useCallback } from 'react';
import Pane from '../Pane';
import { Tabs } from '../layout/Tabs';
import './KnowledgeTab.css';

/* ── Types ────────────────────────────────────────────── */

export interface KnowledgeEntry {
  id: string;
  topic: string;
  summary: string;
  citations: number;
  tier: 'ephemeral' | 'working' | 'durable';
  created_at: number;
}

export interface KnowledgeTabProps {
  entries: KnowledgeEntry[];
  loading?: boolean;
}

/* ── Constants ────────────────────────────────────────── */

const MODE_TABS = [
  { id: 'list', label: 'List' },
  { id: 'graph', label: 'Graph' },
];

const TIER_COLOR: Record<string, string> = {
  ephemeral: '#9a8a98',
  working: '#b4a0c8',
  durable: '#8a9c86',
};

/* ── Component ────────────────────────────────────────── */

/**
 * Knowledge tab: list/graph toggle, search filter, detail panel.
 * Displays knowledge entries from the neuro store.
 */
export function KnowledgeTab({ entries, loading }: KnowledgeTabProps) {
  const [mode, setMode] = useState('list');
  const [search, setSearch] = useState('');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const containerRef = useRef<HTMLDivElement>(null);

  const filtered = entries.filter((e) =>
    !search || e.topic.toLowerCase().includes(search.toLowerCase())
      || e.summary.toLowerCase().includes(search.toLowerCase())
  );

  // Force-directed graph draw
  const drawGraph = useCallback(() => {
    const canvas = canvasRef.current;
    const container = containerRef.current;
    if (!canvas || !container || mode !== 'graph') return;

    const dpr = window.devicePixelRatio || 1;
    const rect = container.getBoundingClientRect();
    const canvasH = 400;
    canvas.width = rect.width * dpr;
    canvas.height = canvasH * dpr;
    canvas.style.width = `${rect.width}px`;
    canvas.style.height = `${canvasH}px`;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;
    ctx.scale(dpr, dpr);

    ctx.fillStyle = '#08080c';
    ctx.fillRect(0, 0, rect.width, canvasH);

    if (filtered.length === 0) {
      ctx.font = '12px JetBrains Mono, monospace';
      ctx.fillStyle = '#9a8a98';
      ctx.textAlign = 'center';
      ctx.fillText('No knowledge entries', rect.width / 2, canvasH / 2);
      return;
    }

    // Simple scatter layout (seeded by index for stability)
    const nodes = filtered.map((e, i) => {
      const angle = (i / Math.max(filtered.length, 1)) * Math.PI * 2;
      const radius = Math.min(rect.width, canvasH) * 0.35;
      return {
        ...e,
        x: rect.width / 2 + Math.cos(angle) * radius * (0.4 + (i % 3) * 0.2),
        y: canvasH / 2 + Math.sin(angle) * radius * (0.4 + (i % 3) * 0.2),
        r: Math.min(20, 4 + e.citations * 2),
      };
    });

    // Draw nodes
    for (const n of nodes) {
      ctx.beginPath();
      ctx.arc(n.x, n.y, n.r, 0, Math.PI * 2);
      ctx.fillStyle = TIER_COLOR[n.tier] ?? '#9a8a98';
      ctx.globalAlpha = n.id === selectedId ? 1 : 0.7;
      ctx.fill();
      ctx.globalAlpha = 1;

      // Selected ring
      if (n.id === selectedId) {
        ctx.strokeStyle = TIER_COLOR[n.tier] ?? '#9a8a98';
        ctx.lineWidth = 2;
        ctx.beginPath();
        ctx.arc(n.x, n.y, n.r + 3, 0, Math.PI * 2);
        ctx.stroke();
      }
    }

    // Legend
    ctx.font = '11px JetBrains Mono, monospace';
    ctx.textAlign = 'left';
    ctx.textBaseline = 'middle';
    const tiers = ['ephemeral', 'working', 'durable'];
    for (let i = 0; i < tiers.length; i++) {
      const lx = 14;
      const ly = canvasH - 50 + i * 16;
      ctx.fillStyle = TIER_COLOR[tiers[i]];
      ctx.beginPath();
      ctx.arc(lx, ly, 4, 0, Math.PI * 2);
      ctx.fill();
      ctx.fillText(tiers[i], lx + 10, ly);
    }
  }, [filtered, mode, selectedId]);

  useEffect(() => { drawGraph(); }, [drawGraph]);

  const selected = selectedId ? entries.find((e) => e.id === selectedId) : null;

  if (loading) {
    return <div className="knowledge-tab__loading">Loading knowledge...</div>;
  }

  return (
    <div className="knowledge-tab">
      <div className="knowledge-tab__header">
        <Tabs tabs={MODE_TABS} active={mode} onChange={setMode} />
        <input
          className="knowledge-tab__search"
          type="text"
          placeholder="Search knowledge..."
          value={search}
          onChange={(e) => setSearch(e.target.value)}
        />
      </div>

      <div className="knowledge-tab__body">
        {mode === 'list' && (
          <div className="knowledge-tab__list">
            {filtered.length === 0 && (
              <p className="knowledge-tab__empty">
                {entries.length === 0 ? 'No knowledge entries yet' : 'No matches'}
              </p>
            )}
            {filtered.map((e) => (
              <div
                key={e.id}
                className={`knowledge-tab__item${selectedId === e.id ? ' knowledge-tab__item--selected' : ''}`}
                onClick={() => setSelectedId(selectedId === e.id ? null : e.id)}
                role="button"
                tabIndex={0}
                onKeyDown={(ev) => {
                  if (ev.key === 'Enter' || ev.key === ' ') {
                    ev.preventDefault();
                    setSelectedId(selectedId === e.id ? null : e.id);
                  }
                }}
              >
                <span className="knowledge-tab__item-topic">{e.topic}</span>
                <span className={`knowledge-tab__item-tier knowledge-tab__item-tier--${e.tier}`}>{e.tier}</span>
                <span className="knowledge-tab__item-citations">{e.citations} cites</span>
              </div>
            ))}
          </div>
        )}

        {mode === 'graph' && (
          <div ref={containerRef} className="knowledge-tab__graph">
            <canvas ref={canvasRef} />
          </div>
        )}
      </div>

      {/* Detail panel */}
      {selected && (
        <Pane title={selected.topic} className="knowledge-tab__detail" flat>
          <p className="knowledge-tab__detail-summary">{selected.summary}</p>
          <div className="knowledge-tab__detail-meta">
            <span>Tier: {selected.tier}</span>
            <span>Citations: {selected.citations}</span>
            <span>Created: {new Date(selected.created_at).toLocaleDateString()}</span>
          </div>
        </Pane>
      )}
    </div>
  );
}
