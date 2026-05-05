import Pane from '../Pane';
import { PhaseRail } from '../layout/PhaseRail';
import './DreamsTab.css';

/* ── Types ────────────────────────────────────────────── */

export interface DreamEntry {
  ts: number;
  type: string;
  summary: string;
}

export interface DreamCycle {
  id: string;
  phase: string;
  progress: number; // 0-1
  started_at: number;
  entries: DreamEntry[];
}

export interface DreamsTabProps {
  cycles: DreamCycle[];
  loading?: boolean;
}

/* ── Constants ────────────────────────────────────────── */

const DREAM_PHASES = ['Hypnagogia', 'Imagine', 'Consolidate', 'Journal', 'Done'];

/* ── Component ────────────────────────────────────────── */

/**
 * Dreams tab: displays dream consolidation cycles with phase rail + entry list.
 * Each cycle shows a PhaseRail and its journal entries.
 */
export function DreamsTab({ cycles, loading }: DreamsTabProps) {
  if (loading) {
    return <div className="dreams-tab__loading">Loading dreams...</div>;
  }

  return (
    <div className="dreams-tab">
      {cycles.length === 0 && (
        <p className="dreams-tab__empty">No dream cycles recorded yet.</p>
      )}

      {cycles.map((c) => {
        const phaseIndex = DREAM_PHASES.indexOf(c.phase);

        return (
          <Pane key={c.id} title={`Cycle ${c.id}`} className="dreams-tab__cycle" flat>
            <PhaseRail
              phases={DREAM_PHASES}
              current={phaseIndex >= 0 ? phaseIndex : 0}
            />

            {c.entries.length > 0 && (
              <div className="dreams-tab__entries">
                {c.entries.map((e, i) => (
                  <div key={i} className="dreams-tab__entry">
                    <span className="dreams-tab__entry-ts">
                      {new Date(e.ts).toLocaleTimeString([], { hour: '2-digit', minute: '2-digit' })}
                    </span>
                    <span className="dreams-tab__entry-type">{e.type}</span>
                    <span className="dreams-tab__entry-summary">{e.summary}</span>
                  </div>
                ))}
              </div>
            )}

            {c.entries.length === 0 && (
              <p className="dreams-tab__no-entries">No entries in this cycle</p>
            )}
          </Pane>
        );
      })}
    </div>
  );
}
