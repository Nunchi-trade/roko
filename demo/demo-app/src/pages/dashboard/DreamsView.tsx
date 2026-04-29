import { useCallback, useEffect, useState } from 'react';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import DreamPhaseViz from '../../components/DreamPhaseViz';
import { useLiveApi } from '../../hooks/useLiveApi';
import { domainColor } from '../../lib/palette';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';

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

  const fetchAll = useCallback(async () => {
    const [j, e] = await Promise.all([
      get<DreamJournal>('/api/dream/journal').catch(() => null),
      get<KnowledgeEntry[]>('/api/knowledge/entries').catch(() => []),
    ]);
    setJournal(j);
    setEntries(Array.isArray(e) ? e : ((e as { items?: KnowledgeEntry[] }).items ?? []));
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
    ['dream_started', 'dream_completed', 'dream_phase_changed'],
    debouncedRefetch,
  );

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
    <div style={{ display: 'flex', flexDirection: 'column', gap: 10 }}>
      {/* ═══ TOP MOSAIC ═══ */}
      <Mosaic columns={6}>
        <MosaicCell label="CYCLES" value={journal?.cycle_count ?? 0} color="dream" mono sub={journal?.last_cycle ? `last: ${new Date(journal.last_cycle).toLocaleDateString()}` : 'none'} />
        <MosaicCell label="PHASES" value={`${completedPhases}/${journal?.phases.length ?? 0}`} color="success" mono />
        <MosaicCell label="EPISODES" value={totalEpisodes} color="rose" mono />
        <MosaicCell label="CLUSTERS" value={totalClusters} color="bone" mono />
        <MosaicCell label="KNOWLEDGE" value={totalKnowledge} color="warning" mono sub="entries written" />
        <MosaicCell label="PLAYBOOKS" value={totalPlaybooks} color="dream" mono />
      </Mosaic>

      {/* ═══ PHASE VISUALIZATION ═══ */}
      <Pane
        title="DREAM PHASE PIPELINE"
        badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>
          {totalDuration > 0 ? `${totalDuration}s total` : 'consolidation cycle'}
        </span>}
      >
        <DreamPhaseViz />
      </Pane>

      {/* ═══ BOTTOM ROW ═══ */}
      <div style={{ display: 'grid', gridTemplateColumns: '1fr 1fr', gap: 10 }}>
        {/* Consolidation Summary */}
        <Pane
          title="CONSOLIDATION SUMMARY"
          badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>per-phase</span>}
        >
          <div style={{ display: 'flex', flexDirection: 'column', gap: 0 }}>
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
                  style={{
                    display: 'flex',
                    alignItems: 'center',
                    justifyContent: 'space-between',
                    gap: 12,
                    padding: '7px 0',
                    borderBottom: i < arr.length - 1 ? '1px solid rgba(255,255,255,.04)' : 'none',
                  }}
                >
                  <span style={{ display: 'flex', alignItems: 'center', gap: 10, flex: 1 }}>
                    <span style={{
                      width: 7, height: 7, borderRadius: '50%',
                      background: isComplete ? color : 'rgba(255,255,255,.15)',
                      boxShadow: isComplete ? `0 0 8px ${color}80` : 'none',
                      display: 'inline-block',
                      flexShrink: 0,
                    }} />
                    <span style={{
                      fontFamily: 'var(--display)',
                      fontSize: 15,
                      fontWeight: 500,
                      color,
                      letterSpacing: '.02em',
                    }}>
                      {phase.name}
                    </span>
                  </span>

                  <span style={{
                    display: 'flex',
                    gap: 14,
                    fontFamily: 'var(--mono)',
                    fontSize: '0.6rem',
                    color: 'var(--text-dim)',
                    letterSpacing: '.04em',
                  }}>
                    <span>{phase.episodes_processed}ep</span>
                    <span>{phase.clusters_formed}cl</span>
                    <span>{phase.knowledge_entries_written}kn</span>
                    <span>{phase.duration_secs}s</span>
                  </span>

                  <span style={{
                    fontFamily: 'var(--mono)',
                    fontSize: 15,
                    padding: '2px 8px',
                    borderRadius: 3,
                    background: isComplete ? `${color}15` : 'rgba(255,255,255,.04)',
                    border: `1px solid ${isComplete ? `${color}30` : 'rgba(255,255,255,.06)'}`,
                    color: isComplete ? color : 'var(--text-ghost)',
                    letterSpacing: '.06em',
                  }}>
                    {phase.status}
                  </span>
                </div>
              );
            })}
          </div>
        </Pane>

        {/* Recent Knowledge Artifacts */}
        <Pane
          title="RECENT DREAM ARTIFACTS"
          badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 13 }}>knowledge entries</span>}
        >
          {recentEntries.length === 0 ? (
            <div style={{
              padding: 36,
              color: 'var(--text-ghost)',
              fontFamily: 'var(--mono)',
              fontSize: '0.75rem',
              textAlign: 'center',
            }}>
              Artifacts emerge after dream consolidation cycles
            </div>
          ) : (
            <div style={{ display: 'flex', flexDirection: 'column', gap: 0 }}>
              {recentEntries.map((entry, i) => {
                const color = domainColor(entry.domain);

                return (
                  <div
                    key={entry.id}
                    style={{
                      display: 'flex',
                      alignItems: 'center',
                      gap: 10,
                      padding: '5px 0',
                      borderBottom: i < recentEntries.length - 1 ? '1px solid rgba(255,255,255,.04)' : 'none',
                    }}
                  >
                    <span style={{
                      width: 5, height: 5, borderRadius: '50%',
                      background: color,
                      boxShadow: `0 0 6px ${color}60`,
                      display: 'inline-block',
                      flexShrink: 0,
                    }} />
                    <span style={{
                      fontFamily: 'var(--mono)',
                      fontSize: 14,
                      color: 'var(--text-primary)',
                      flex: 1,
                      overflow: 'hidden',
                      textOverflow: 'ellipsis',
                      whiteSpace: 'nowrap',
                    }}>
                      {entry.label ?? entry.id}
                    </span>
                    <span style={{
                      fontFamily: 'var(--mono)',
                      fontSize: 15,
                      color: 'var(--text-ghost)',
                      letterSpacing: '.06em',
                      flexShrink: 0,
                    }}>
                      {entry.domain ?? 'unknown'}
                    </span>
                    {entry.citations != null && (
                      <span style={{
                        fontFamily: 'var(--mono)',
                        fontSize: 15,
                        color: 'var(--bone-bright)',
                        flexShrink: 0,
                      }}>
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
  );
}
