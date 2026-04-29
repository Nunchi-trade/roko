// --- src/scenes/Knowledge.tsx ---
// T3.15: Knowledge scene — full-page graph/list explorer + dream cycles
import { useState, useEffect, useRef, useCallback } from 'react';
import { Tabs } from '../components/layout/Tabs';
import { SplitView } from '../components/layout/SplitView';
import Pane from '../components/Pane';
import { PhaseRail } from '../components/layout/PhaseRail';
import { useCanvasSetup } from '../hooks/useCanvasSetup';
import './Knowledge.css';

interface KnowledgeEntry {
  id: string;
  topic: string;
  summary: string;
  citations: number;
  tier: 'ephemeral' | 'working' | 'durable';
  created_at: number;
  related?: string[];
}

interface DreamCycle {
  id: string;
  phase: string;
  entries: Array<{ type: string; summary: string }>;
}

const API_BASE = '/api';
const TABS = [
  { id: 'explore', label: 'Explore' },
  { id: 'dreams', label: 'Dreams' },
];
const DREAM_PHASES = ['Hypnagogia', 'Imagine', 'Consolidate', 'Journal', 'Done'];

const TIER_COLOR: Record<string, string> = {
  ephemeral: '#8a8078',
  working: '#b4a0c8',
  durable: '#8cb48c',
};

const LABEL_COLOR = '#d4c8b8';
const BG_COLOR = '#0a0908';

export function Knowledge() {
  const [activeTab, setActiveTab] = useState('explore');
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [dreams, setDreams] = useState<DreamCycle[]>([]);
  const [search, setSearch] = useState('');
  const [mode, setMode] = useState<'graph' | 'list'>('graph');
  const [selectedId, setSelectedId] = useState<string | null>(null);
  const canvasRef = useRef<HTMLCanvasElement>(null);

  // -- Stable positions for nodes so the graph doesn't re-randomize on every render --
  const positionsRef = useRef<Map<string, { x: number; y: number }>>(new Map());

  useEffect(() => {
    let active = true;
    async function load() {
      try {
        const [kRes, dRes] = await Promise.all([
          fetch(`${API_BASE}/knowledge?limit=1000`),
          fetch(`${API_BASE}/knowledge/dreams`),
        ]);
        if (kRes.ok && active) setEntries(await kRes.json());
        if (dRes.ok && active) setDreams(await dRes.json());
      } catch { /* API may not be available */ }
    }
    load();
    return () => { active = false; };
  }, []);

  const filtered = entries.filter((e) =>
    !search || e.topic.toLowerCase().includes(search.toLowerCase())
      || e.summary.toLowerCase().includes(search.toLowerCase())
  );

  const selected = selectedId ? entries.find((e) => e.id === selectedId) : null;

  // -- Force-directed graph draw callback (used by useCanvasSetup) --
  const drawGraph = useCallback((ctx: CanvasRenderingContext2D, w: number, h: number) => {
    ctx.fillStyle = BG_COLOR;
    ctx.fillRect(0, 0, w, h);

    if (filtered.length === 0) {
      ctx.fillStyle = LABEL_COLOR;
      ctx.font = '13px monospace';
      ctx.textAlign = 'center';
      ctx.fillText('No knowledge entries', w / 2, h / 2);
      return;
    }

    // Assign stable random positions per entry id
    const positions = positionsRef.current;
    for (const e of filtered) {
      if (!positions.has(e.id)) {
        positions.set(e.id, {
          x: w * 0.15 + Math.random() * w * 0.7,
          y: h * 0.15 + Math.random() * h * 0.7,
        });
      }
    }

    const nodes = filtered.map((e) => {
      const pos = positions.get(e.id)!;
      // Clamp positions within current canvas bounds
      const cx = Math.min(Math.max(pos.x, 30), w - 30);
      const cy = Math.min(Math.max(pos.y, 30), h - 30);
      return {
        ...e,
        x: cx,
        y: cy,
        r: Math.min(24, 5 + e.citations * 2),
      };
    });

    // Draw connection lines for related entries
    ctx.strokeStyle = 'rgba(255,255,255,0.06)';
    ctx.lineWidth = 1;
    const nodeMap = new Map(nodes.map((n) => [n.id, n]));
    for (const n of nodes) {
      for (const rid of n.related ?? []) {
        const target = nodeMap.get(rid);
        if (target) {
          ctx.beginPath();
          ctx.moveTo(n.x, n.y);
          ctx.lineTo(target.x, target.y);
          ctx.stroke();
        }
      }
    }

    // Draw nodes
    for (const n of nodes) {
      const isSelected = n.id === selectedId;

      // Glow for selected node
      if (isSelected) {
        ctx.shadowColor = TIER_COLOR[n.tier] ?? TIER_COLOR.ephemeral;
        ctx.shadowBlur = 14;
      }

      ctx.beginPath();
      ctx.arc(n.x, n.y, n.r, 0, Math.PI * 2);
      ctx.fillStyle = TIER_COLOR[n.tier] ?? TIER_COLOR.ephemeral;
      ctx.globalAlpha = isSelected ? 1 : 0.7;
      ctx.fill();
      ctx.globalAlpha = 1;
      ctx.shadowBlur = 0;

      // Label for larger nodes
      if (n.r > 10) {
        ctx.fillStyle = LABEL_COLOR;
        ctx.font = '11px monospace';
        ctx.textAlign = 'center';
        ctx.fillText(n.topic.slice(0, 15), n.x, n.y + n.r + 12);
      }
    }
  }, [filtered, selectedId]);

  // Use the shared canvas hook for DPR-safe drawing + ResizeObserver
  useCanvasSetup(canvasRef, drawGraph, [filtered, selectedId, mode]);

  // -- Canvas click handler to select nodes --
  const handleCanvasClick = useCallback((e: React.MouseEvent<HTMLCanvasElement>) => {
    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const mx = e.clientX - rect.left;
    const my = e.clientY - rect.top;

    const positions = positionsRef.current;
    for (const entry of filtered) {
      const pos = positions.get(entry.id);
      if (!pos) continue;
      const r = Math.min(24, 5 + entry.citations * 2);
      // Clamp to match draw
      const cx = Math.min(Math.max(pos.x, 30), rect.width - 30);
      const cy = Math.min(Math.max(pos.y, 30), rect.height - 30);
      const dx = mx - cx;
      const dy = my - cy;
      if (dx * dx + dy * dy <= r * r) {
        setSelectedId(entry.id);
        return;
      }
    }
    // Clicked empty space
    setSelectedId(null);
  }, [filtered]);

  const exploreContent = (
    <SplitView
      left={
        <div className="knowledge__main">
          <div className="knowledge__toolbar">
            <button
              className={`knowledge__mode-btn ${mode === 'graph' ? 'knowledge__mode-btn--active' : ''}`}
              onClick={() => setMode('graph')}
            >Graph</button>
            <button
              className={`knowledge__mode-btn ${mode === 'list' ? 'knowledge__mode-btn--active' : ''}`}
              onClick={() => setMode('list')}
            >List</button>
            <input
              className="knowledge__search"
              type="text"
              placeholder="Search..."
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </div>

          {mode === 'graph' && (
            <div className="knowledge__graph">
              <canvas
                ref={canvasRef}
                onClick={handleCanvasClick}
                style={{ cursor: 'pointer' }}
              />
            </div>
          )}

          {mode === 'list' && (
            <div className="knowledge__list">
              {filtered.map((e) => (
                <div
                  key={e.id}
                  className={`knowledge__item ${selectedId === e.id ? 'knowledge__item--selected' : ''}`}
                  onClick={() => setSelectedId(e.id)}
                >
                  <span className="knowledge__item-topic">{e.topic}</span>
                  <span className={`knowledge__item-tier knowledge__item-tier--${e.tier}`}>{e.tier}</span>
                  <span className="knowledge__item-citations">{e.citations}</span>
                </div>
              ))}
            </div>
          )}
        </div>
      }
      right={
        selected ? (
          <Pane title={selected.topic} flat>
            <div className="knowledge__detail">
              <p className="knowledge__detail-summary">{selected.summary}</p>
              <div className="knowledge__detail-meta">
                <span>Tier: {selected.tier}</span>
                <span>Citations: {selected.citations}</span>
                <span>Created: {new Date(selected.created_at).toLocaleDateString()}</span>
              </div>
              {selected.related && selected.related.length > 0 && (
                <div className="knowledge__detail-related">
                  <h4>Related</h4>
                  {selected.related.map((rid) => {
                    const rel = entries.find((e) => e.id === rid);
                    return rel ? (
                      <div key={rid} className="knowledge__related-item" onClick={() => setSelectedId(rid)}>
                        {rel.topic}
                      </div>
                    ) : null;
                  })}
                </div>
              )}
            </div>
          </Pane>
        ) : (
          <Pane title="Detail" flat>
            <p style={{ color: 'var(--text-dim)', padding: 20 }}>Select an entry to view details</p>
          </Pane>
        )
      }
      defaultSplit={65}
    />
  );

  const dreamsContent = (
    <div className="knowledge__dreams">
      {dreams.length === 0 && (
        <p style={{ color: 'var(--text-dim)', padding: 40, textAlign: 'center' }}>
          No dream cycles recorded yet.
        </p>
      )}
      {dreams.map((c) => (
        <Pane key={c.id} title={`Cycle ${c.id}`} flat>
          <PhaseRail phases={DREAM_PHASES} current={DREAM_PHASES.indexOf(c.phase)} />
          <div className="knowledge__dream-entries">
            {c.entries.map((e, i) => (
              <div key={i} className="knowledge__dream-entry">
                <span className="knowledge__dream-type">{e.type}</span>
                <span className="knowledge__dream-summary">{e.summary}</span>
              </div>
            ))}
          </div>
        </Pane>
      ))}
    </div>
  );

  return (
    <div className="knowledge">
      <Tabs tabs={TABS} active={activeTab} onChange={setActiveTab} />
      <div className="knowledge__body">
        {activeTab === 'explore' && exploreContent}
        {activeTab === 'dreams' && dreamsContent}
      </div>
    </div>
  );
}
