import { useCallback, useEffect, useState } from 'react';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import DreamPhaseViz from '../../components/DreamPhaseViz';
import { useLiveApi } from '../../hooks/useLiveApi';
import { domainColor } from '../../lib/palette';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import './dashboard.css';

/* ── Types ────────────────────────────────────────────────── */

interface DreamPhase {
  name: string;
  status: string;
  episodes_processed: number;
  clusters_formed: number;
  knowledge_entries_written: number;
  playbooks_created: number;
  duration_secs: number;
  trend: number[];
}

interface DreamJournal {
  last_cycle: string;
  cycle_count: number;
  phases: DreamPhase[];
}

interface KnowledgeEntry {
  id: string;
  domain?: string;
  label?: string;
  citations?: number;
}

/* ── Component ───────────────────────────────────────────── */

export default function DreamsView() {
  const { get } = useLiveApi();
  const [journal, setJournal] = useState<DreamJournal | null>(null);
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);
  const [initialLoading, setInitialLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);

  const fetchAll = useCallback(async () => {
    try {
      const [j, e] = await Promise.all([
        get<DreamJournal>('/api/dream/journal').catch(() => null),
        get<KnowledgeEntry[]>('/api/knowledge/entries').catch(() => []),
      ]);
      setJournal(j);
      setEntries(Array.isArray(e) ? e : ((e as { items?: KnowledgeEntry[] }).items ?? []));
      setError(null);
    } catch (err) {
      setError(err instanceof Error ? err.message : 'Failed to load dreams data');
    } finally {
      setInitialLoading(false);
    }
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
    ['dream_started', 'dream_completed', 'dream_phase_changed', 'knowledge_ingested', 'knowledge_consumed'],
    debouncedRefetch,
  );

  if (initialLoading) {
    return (
      <div className="dash-page progressive-reveal">
        <div className="skeleton" style={{ height: 32, borderRadius: 6 }} />
        <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 8, marginTop: 8 }}>
          <div className="skeleton" style={{ height: 200, borderRadius: 6 }} />
          <div className="skeleton" style={{ height: 200, borderRadius: 6 }} />
        </div>
      </div>
    );
  }

  /* Derived stats */
  const totalEpisodes = journal?.phases.reduce((s, p) => s + p.episodes_processed, 0) ?? 0;
  const totalClusters = journal?.phases.reduce((s, p) => s + p.clusters_formed, 0) ?? 0;
  const totalKnowledge = journal?.phases.reduce((s, p) => s + p.knowledge_entries_written, 0) ?? 0;
  const totalPlaybooks = journal?.phases.reduce((s, p) => s + p.playbooks_created, 0) ?? 0;
  const completedPhases = journal?.phases.filter((p) => p.status === 'completed').length ?? 0;
  const totalDuration = journal?.phases.reduce((s, p) => s + p.duration_secs, 0) ?? 0;

  /* Recent knowledge entries (last 8) as dream artifacts */
  const recentEntries = entries.slice(-8).reverse();

  return (
    <div className="dash-page" style={{ animation: 'fadeInUp 0.35s var(--ease) both' }}>
      {error && (
        <div style={{ padding: '8px 12px', background: 'var(--rose-deep)', border: '1px solid var(--rose-dim)', borderRadius: 'var(--radius-md)', fontFamily: 'var(--mono)', fontSize: 'var(--text-xs)', color: 'var(--rose-bright)', marginBottom: 8 }}>
          {error}
        </div>
      )}
      {/* TOP MOSAIC */}
      <div className="dash-stagger" style={{ '--stagger-i': 0 } as React.CSSProperties}>
        <Mosaic columns={6}>
          <MosaicCell label="CYCLES" value={journal?.cycle_count ?? 0} color="dream" mono sub={journal?.last_cycle ? `last: ${new Date(journal.last_cycle).toLocaleDateString()}` : 'none'} />
          <MosaicCell label="PHASES" value={`${completedPhases}/${journal?.phases.length ?? 0}`} color="success" mono />
          <MosaicCell label="EPISODES" value={totalEpisodes} color="rose" mono />
          <MosaicCell label="CLUSTERS" value={totalClusters} color="bone" mono />
          <MosaicCell label="KNOWLEDGE" value={totalKnowledge} color="warning" mono sub="entries written" />
          <MosaicCell label="PLAYBOOKS" value={totalPlaybooks} color="dream" mono />
        </Mosaic>
      </div>

      {/* PHASE VISUALIZATION */}
      <div className="dash-stagger" style={{ '--stagger-i': 1 } as React.CSSProperties}>
        <Pane
          title="DREAM PHASE PIPELINE"
          badge={<span className="dash-badge">
            {totalDuration > 0 ? `${totalDuration}s total` : 'consolidation cycle'}
          </span>}
        >
          <div className="dash-chart-enter">
            <DreamPhaseViz />
          </div>
        </Pane>
      </div>

      {/* BOTTOM ROW */}
      <div className="dash-grid-2">
        {/* Consolidation Summary */}
        <div className="dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
          <Pane
            title="CONSOLIDATION SUMMARY"
            badge={<span className="dash-badge">per-phase</span>}
          >
            <div className="dash-flex-col">
              {(journal?.phases ?? []).map((phase, i, arr) => {
                const phaseColors: Record<string, string> = {
                  Hypnagogia: 'var(--dream-bright)',
                  NREM: 'var(--dream)',
                  REM: 'var(--rose-bright)',
                  Integration: 'var(--success)',
                };
                const color = phaseColors[phase.name] ?? 'var(--text-ghost)';
                const isComplete = phase.status === 'completed';

                return (
                  <div
                    key={phase.name}
                    className={`dash-consolidation-row dash-dream-reveal${i < arr.length - 1 ? ' dash-row-sep' : ''}`}
                    style={{ animationDelay: `${i * 120 + 200}ms` }}
                  >
                    <span className="dash-inline--10">
                      <span
                        className="dash-dot--7 dash-status-breathe"
                        style={{
                          background: isComplete ? color : 'rgba(255,255,255,.15)',
                          color: isComplete ? color : 'rgba(255,255,255,.15)',
                          boxShadow: isComplete ? `0 0 8px ${color}80` : 'none',
                        }}
                      />
                      <span className="dash-display-label" style={{ color }}>
                        {phase.name}
                      </span>
                    </span>

                    <span className="dash-phase-stats">
                      <span>{phase.episodes_processed}ep</span>
                      <span>{phase.clusters_formed}cl</span>
                      <span>{phase.knowledge_entries_written}kn</span>
                      <span>{phase.duration_secs}s</span>
                    </span>

                    <span
                      className="dash-status-badge"
                      style={{
                        background: isComplete ? `${color}15` : 'rgba(255,255,255,.04)',
                        border: `1px solid ${isComplete ? `${color}30` : 'rgba(255,255,255,.06)'}`,
                        color: isComplete ? color : 'var(--text-ghost)',
                      }}
                    >
                      {phase.status}
                    </span>
                  </div>
                );
              })}
            </div>
          </Pane>
        </div>

        {/* Recent Knowledge Artifacts */}
        <div className="dash-stagger" style={{ '--stagger-i': 3 } as React.CSSProperties}>
          <Pane
            title="RECENT DREAM ARTIFACTS"
            badge={<span className="dash-badge">knowledge entries</span>}
          >
            {recentEntries.length === 0 ? (
              <div className="dash-placeholder--lg">
                Artifacts emerge after dream consolidation cycles
              </div>
            ) : (
              <div className="dash-flex-col">
                {recentEntries.map((entry, i) => {
                  const color = domainColor(entry.domain);

                  return (
                    <div
                      key={entry.id}
                      className={`dash-row-item dash-dream-reveal${i < recentEntries.length - 1 ? ' dash-row-sep' : ''}`}
                      style={{ gap: 10, padding: '5px 0', animationDelay: `${i * 80 + 300}ms` }}
                    >
                      <span
                        className="dash-dot--5"
                        style={{
                          background: color,
                          boxShadow: `0 0 6px ${color}60`,
                        }}
                      />
                      <span className="dash-ellipsis">
                        {entry.label ?? entry.id}
                      </span>
                      <span className="dash-ghost">
                        {entry.domain ?? 'unknown'}
                      </span>
                      {entry.citations != null && (
                        <span className="dash-bone">
                          {entry.citations}
                        </span>
                      )}
                    </div>
                  );
                })}
              </div>
            )}
          </Pane>
        </div>
      </div>
    </div>
  );
}
