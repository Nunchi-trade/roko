import { useState, useEffect, useCallback } from 'react';
import { useLiveApi } from '../../hooks/useLiveApi';
import { DOMAIN_COLORS, domainColor } from '../../lib/palette';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import KnowledgeOrbit, { type KnowledgeNode as OrbitNode, type KnowledgeEdge as OrbitEdge } from '../../components/three/KnowledgeOrbit';
import './dashboard.css';
import './KnowledgeGraph.css';

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

/* ── Component ───────────────────────────────────────────── */

export default function KnowledgeGraph() {
  const { get } = useLiveApi();
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [edges, setEdges] = useState<KnowledgeEdge[]>([]);

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

  /* Map API data to KnowledgeOrbit types */
  const VALID_DOMAINS = new Set(['gate', 'agent', 'knowledge', 'plan', 'config']);
  const orbitNodes: OrbitNode[] = entries.map((e) => ({
    id: e.id,
    label: e.label ?? e.id,
    domain: (VALID_DOMAINS.has(e.domain ?? '') ? e.domain : 'knowledge') as OrbitNode['domain'],
    weight: e.citations ?? 1,
  }));
  const orbitEdges: OrbitEdge[] = edges.map((e) => ({
    source: e.source,
    target: e.target,
    strength: e.frequency,
  }));

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
    <div className="dash-page">
      {/* TOP MOSAIC */}
      <div className="dash-stagger" style={{ '--stagger-i': 0 } as React.CSSProperties}>
        <Mosaic columns={5}>
          <MosaicCell label="NODES" value={entries.length} color="rose" mono />
          <MosaicCell label="EDGES" value={edges.length} color="bone" mono />
          <MosaicCell label="DOMAINS" value={domains.size} color="dream" mono />
          <MosaicCell label="CITATIONS" value={totalCitations} color="warning" mono />
          <MosaicCell label="AVG CITATIONS" value={avgCitations.toFixed(1)} color="success" mono />
        </Mosaic>
      </div>

      {/* GRAPH + DOMAIN BREAKDOWN */}
      <div className="dash-grid-2-1">
        <div className="dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
          <Pane title="KNOWLEDGE GRAPH" badge={<span className="dash-badge">3D orbital</span>}>
            <div className="dash-relative dash-chart-enter kg-canvas-wrap">
              <KnowledgeOrbit nodes={orbitNodes} edges={orbitEdges} height={260} />
              {/* HUD overlays */}
              <div className="dash-hud-tl">
                {entries.length} NODES / {edges.length} EDGES
              </div>
              <div className="dash-hud-tr">
                {Object.entries(DOMAIN_COLORS).map(([d, c]) => (
                  <span key={d} className="dash-hud-legend">
                    <span
                      className="dash-dot--5"
                      style={{ background: c, boxShadow: `0 0 4px ${c}80` }}
                    />
                    {d}
                  </span>
                ))}
              </div>
              <div className="dash-hud-bl">
                ORBITAL / 3D
              </div>
            </div>
          </Pane>
        </div>

        {/* Domain Breakdown */}
        <div className="dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
          <Pane title="DOMAIN BREAKDOWN" badge={<span className="dash-badge">{sortedDomains.length} domains</span>}>
            <div className="dash-flex-col">
              {sortedDomains.map(([domain, count], i) => {
                const total = entries.length || 1;
                const pct = (count / total) * 100;
                const color = domainColor(domain);

                return (
                  <div
                    key={domain}
                    className={`dash-domain-row${i < sortedDomains.length - 1 ? ' dash-row-sep' : ''} dash-stagger`}
                    style={{ '--stagger-i': i + 3 } as React.CSSProperties}
                  >
                    <div className="dash-row-item--between">
                      <span className="dash-inline">
                        <span
                          className="dash-dot"
                          style={{ background: color, boxShadow: `0 0 6px ${color}60` }}
                        />
                        <span className="dash-display-label" style={{ color }}>
                          {domain}
                        </span>
                      </span>
                      <span className="dash-domain-count">{count}</span>
                    </div>
                    <div className="dash-bar-track">
                      <div
                        className="dash-bar-fill dash-bar-animate kg-bar-fill"
                        style={{
                          width: `${pct}%`,
                          background: color,
                          boxShadow: `0 0 8px ${color}40`,
                          animationDelay: `${i * 80 + 300}ms`,
                        }}
                      />
                    </div>
                    <span className="dash-domain-pct">
                      {pct.toFixed(0)}% of graph
                    </span>
                  </div>
                );
              })}
            </div>
          </Pane>
        </div>
      </div>

      {/* ENTRIES TABLE */}
      <div className="dash-stagger" style={{ '--stagger-i': 3 } as React.CSSProperties}>
        <Pane title="ALL ENTRIES" badge={<span className="dash-badge">{entries.length} nodes</span>} flat>
          <div className="dash-table-scroll">
            <table className="dash-table">
              <thead>
                <tr>
                  {['ID', 'DOMAIN', 'LABEL', 'CITATIONS'].map((h) => (
                    <th key={h}>{h}</th>
                  ))}
                </tr>
              </thead>
              <tbody>
                {entries.map((e, rowIdx) => (
                  <tr
                    key={e.id}
                    className="dash-table-row dash-row-sep--light dash-stagger kg-entry-row"
                    style={{ '--stagger-i': rowIdx } as React.CSSProperties}
                  >
                    <td className="kg-cell-id">{e.id}</td>
                    <td>
                      <span className="dash-inline" style={{ color: domainColor(e.domain) }}>
                        <span
                          className="dash-dot--5"
                          style={{
                            background: domainColor(e.domain),
                            boxShadow: `0 0 4px ${domainColor(e.domain)}60`,
                          }}
                        />
                        {e.domain ?? 'unknown'}
                      </span>
                    </td>
                    <td className="kg-cell-label">{e.label ?? '-'}</td>
                    <td className="kg-cell-citations">{e.citations ?? 0}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Pane>
      </div>
    </div>
  );
}
