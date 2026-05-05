/**
 * CompareTab — side-by-side run comparison with config diff + radar chart.
 * Extracted from the Compare section of Bench.tsx (lines 562-618).
 */
import type { BenchRun } from '../../lib/bench-types';
import Pane from '../Pane';
import ConfigDiff from '../ConfigDiff';
import RadarChart from '../Charts/RadarChart';
import './CompareTab.css';

/* ── Constants ── */

const RUN_COLORS = [
  'var(--rose-bright)',
  'var(--bone-bright)',
  'var(--success)',
  'var(--dream-bright)',
  'var(--dream)',
  'var(--warning)',
];

/* ── Props ── */

export interface CompareTabProps {
  history: BenchRun[];
  compareIds: string[];
  setCompareIds: (ids: string[]) => void;
}

/* ── Component ── */

export function CompareTab({ history, compareIds, setCompareIds }: CompareTabProps) {
  const compareRuns = history.filter((r) => compareIds.includes(r.id));

  return (
    <div className="compare-tab">
      <Pane title="SELECT RUNS TO COMPARE">
        <div className="bench-compare-chips">
          {compareIds.map((id) => {
            const run = history.find((r) => r.id === id);
            return (
              <div key={id} className="bench-chip">
                <span>
                  {run
                    ? `${run.id.slice(0, 8)} \u00B7 ${run.suite_name}`
                    : id.slice(0, 8)}
                </span>
                <button
                  className="bench-chip-x"
                  onClick={() =>
                    setCompareIds(compareIds.filter((x) => x !== id))
                  }
                >
                  &times;
                </button>
              </div>
            );
          })}
          {compareIds.length < 6 && (
            <select
              className="config-input"
              style={{ maxWidth: 200 }}
              value=""
              onChange={(e) => {
                if (e.target.value && !compareIds.includes(e.target.value))
                  setCompareIds([...compareIds, e.target.value]);
              }}
            >
              <option value="">Add run...</option>
              {history
                .filter((r) => !compareIds.includes(r.id))
                .map((r) => (
                  <option key={r.id} value={r.id}>
                    {r.id.slice(0, 8)} -{' '}
                    {r.suite_name} (
                    {r.config.model.split('-').slice(0, 2).join('-')})
                  </option>
                ))}
            </select>
          )}
        </div>
        <div className="bench-compare-quick">
          <button
            className="btn btn-sm"
            onClick={() => {
              if (history.length >= 2)
                setCompareIds(history.slice(0, 2).map((r) => r.id));
            }}
          >
            Last 2
          </button>
          <button
            className="btn btn-sm"
            onClick={() => {
              const sid = history[0]?.suite_id;
              if (sid)
                setCompareIds(
                  history
                    .filter((r) => r.suite_id === sid)
                    .slice(0, 4)
                    .map((r) => r.id),
                );
            }}
          >
            Same Suite
          </button>
          <button
            className="btn btn-sm"
            onClick={() => {
              const m = history[0]?.config.model;
              if (m)
                setCompareIds(
                  history
                    .filter((r) => r.config.model === m)
                    .slice(0, 4)
                    .map((r) => r.id),
                );
            }}
          >
            Same Model
          </button>
        </div>
      </Pane>

      {compareRuns.length >= 2 ? (
        <>
          <Pane title="CONFIG DIFF">
            <ConfigDiff runs={compareRuns} />
          </Pane>

          {(() => {
            const axes = [
              'Pass Rate',
              'Speed',
              'Cost Eff.',
              'Token Eff.',
              'Gate Pass',
            ];
            const datasets = compareRuns
              .map((run, i) => {
                const s = run.summary;
                if (!s) return null;
                let gp = 0;
                let gt = 0;
                for (const res of run.results)
                  for (const g of res.gate_verdicts) {
                    gt++;
                    if (g.passed) gp++;
                  }
                return {
                  label: run.config.model.split('-').slice(0, 2).join('-'),
                  values: [
                    s.pass_rate,
                    1 - Math.min(s.avg_duration_ms / 60000, 1),
                    1 - Math.min(s.total_cost_usd / 1, 1),
                    Math.min(
                      s.passed / Math.max(s.total_tokens / 1000, 0.001),
                      1,
                    ),
                    gt > 0 ? gp / gt : 0,
                  ],
                  color: RUN_COLORS[i % RUN_COLORS.length],
                };
              })
              .filter(
                (d): d is NonNullable<typeof d> => d != null,
              );
            return datasets.length >= 2 ? (
              <Pane title="RADAR COMPARISON">
                <RadarChart axes={axes} datasets={datasets} height={350} />
              </Pane>
            ) : null;
          })()}
        </>
      ) : (
        <div className="bench-empty--no-runs">
          <p className="bench-empty-text">
            Select at least 2 runs to compare.
          </p>
        </div>
      )}
    </div>
  );
}
