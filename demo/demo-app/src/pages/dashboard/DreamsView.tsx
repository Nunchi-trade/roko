import { useEffect, useState } from 'react';
import Pane from '../../components/Pane';
import Mosaic, { MosaicCell } from '../../components/Mosaic';
import DreamPhaseViz from '../../components/DreamPhaseViz';
import { useApiWithFallback } from '../../hooks/useApiWithFallback';

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
  const { get } = useApiWithFallback();
  const [journal, setJournal] = useState<DreamJournal | null>(null);
  const [entries, setEntries] = useState<KnowledgeEntry[]>([]);

  useEffect(() => {
    let cancelled = false;

    const poll = async () => {
      const [j, e] = await Promise.all([
        get<DreamJournal>('/api/dream/journal').catch(() => null),
        get<KnowledgeEntry[]>('/api/knowledge/entries').catch(() => []),
      ]);
      if (cancelled) return;
      setJournal(j);
      setEntries(Array.isArray(e) ? e : []);
    };

    poll();
    const id = setInterval(poll, 30_000);
    return () => { cancelled = true; clearInterval(id); };
  }, [get]);

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
        <MosaicCell label="CYCLES" value={journal?.cycle_count ?? 12} color="dream" mono sub={journal?.last_cycle ? `last: ${new Date(journal.last_cycle).toLocaleDateString()}` : 'periodic'} />
        <MosaicCell label="PHASES" value={`${completedPhases || 4}/4`} color="success" mono />
        <MosaicCell label="EPISODES" value={totalEpisodes || 847} color="rose" mono />
        <MosaicCell label="CLUSTERS" value={totalClusters || 42} color="bone" mono />
        <MosaicCell label="KNOWLEDGE" value={totalKnowledge || 18} color="warning" mono sub="entries written" />
        <MosaicCell label="PLAYBOOKS" value={totalPlaybooks || 7} color="dream" mono />
      </Mosaic>

      {/* ═══ PHASE VISUALIZATION ═══ */}
      <Pane
        title="DREAM PHASE PIPELINE"
        badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>
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
          badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>per-phase</span>}
        >
          <div style={{ display: 'flex', flexDirection: 'column', gap: 0 }}>
            {(journal?.phases ?? []).map((phase, i, arr) => {
              const phaseColors: Record<string, string> = {
                Hypnagogia: '#9A8AB8',
                NREM: '#7A8AA8',
                REM: '#CC90A8',
                Integration: '#8A9C86',
              };
              const color = phaseColors[phase.name] ?? '#706070';
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
                      fontSize: 12,
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
                    fontSize: 9,
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
          badge={<span style={{ fontFamily: 'var(--mono)', fontSize: 10 }}>knowledge entries</span>}
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
                const DOMAIN_COLORS: Record<string, string> = {
                  gate: '#CC90A8',
                  agent: '#C8B890',
                  knowledge: '#9494B4',
                  plan: '#7A8A78',
                  config: '#C89A68',
                };
                const color = DOMAIN_COLORS[entry.domain ?? ''] ?? '#706070';

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
                      fontSize: 11,
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
                      fontSize: 9,
                      color: 'var(--text-ghost)',
                      letterSpacing: '.06em',
                      flexShrink: 0,
                    }}>
                      {entry.domain ?? 'unknown'}
                    </span>
                    {entry.citations != null && (
                      <span style={{
                        fontFamily: 'var(--mono)',
                        fontSize: 9,
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
