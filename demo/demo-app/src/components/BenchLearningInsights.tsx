import { useState, useEffect, useMemo } from 'react';
import { useLiveApi } from '../hooks/useLiveApi';
import { shortModel } from '../lib/format';
import type { BenchLearningEvent, BenchRun } from '../lib/bench-types';
import Pane from './Pane';
import Mosaic, { MosaicCell } from './Mosaic';

/* ── Types for learning API responses ── */

interface CascadeRouterModel {
  model: string;
  provider?: string;
  trials: number;
  successes: number;
  avg_cost?: number;
}

interface CascadeRouterResponse {
  observations: number;
  models?: CascadeRouterModel[];
  model_stats?: CascadeRouterModel[];
}

interface GateThreshold {
  gate: string;
  threshold: number;
  observations: number;
}

interface GateThresholdsResponse {
  thresholds?: GateThreshold[];
  rungs?: Record<string, GateThreshold[]>;
}

interface EfficiencyResponse {
  total_events?: number;
  avg_tokens_per_turn?: number;
  avg_cost_per_turn?: number;
}

/* ── Props ── */

interface BenchLearningInsightsProps {
  history: BenchRun[];
  learningEvents: BenchLearningEvent[];
  isRunning: boolean;
}

/* ── Recommendation engine (client-side) ── */

interface Recommendation {
  text: string;
  confidence: number;
  type: 'cost' | 'quality' | 'speed';
}

function computeRecommendations(history: BenchRun[]): Recommendation[] {
  const recs: Recommendation[] = [];
  const completed = history.filter((r) => r.status === 'completed' && r.summary);
  if (completed.length < 2) return recs;

  // Group by model
  const byModel = new Map<string, { runs: number; avgPass: number; avgCost: number; avgDur: number }>();
  for (const run of completed) {
    const s = run.summary!;
    const key = run.config.model;
    const prev = byModel.get(key) ?? { runs: 0, avgPass: 0, avgCost: 0, avgDur: 0 };
    const n = prev.runs + 1;
    byModel.set(key, {
      runs: n,
      avgPass: (prev.avgPass * prev.runs + s.pass_rate) / n,
      avgCost: (prev.avgCost * prev.runs + s.total_cost_usd) / n,
      avgDur: (prev.avgDur * prev.runs + s.avg_duration_ms) / n,
    });
  }

  const models = [...byModel.entries()].sort((a, b) => b[1].avgPass - a[1].avgPass);

  // Cost savings recommendation
  if (models.length >= 2) {
    const best = models[0];
    const cheapest = [...models].sort((a, b) => a[1].avgCost - b[1].avgCost)[0];
    if (cheapest[0] !== best[0] && cheapest[1].avgPass >= best[1].avgPass * 0.9) {
      const savings = ((1 - cheapest[1].avgCost / best[1].avgCost) * 100).toFixed(0);
      recs.push({
        text: `${shortModel(cheapest[0])} is ${savings}% cheaper than ${shortModel(best[0])} with comparable pass rate (${(cheapest[1].avgPass * 100).toFixed(0)}% vs ${(best[1].avgPass * 100).toFixed(0)}%)`,
        confidence: Math.min(cheapest[1].runs, best[1].runs) / 5,
        type: 'cost',
      });
    }
  }

  // Best model recommendation
  if (models.length >= 1 && models[0][1].runs >= 2) {
    recs.push({
      text: `${shortModel(models[0][0])} leads with ${(models[0][1].avgPass * 100).toFixed(0)}% pass rate across ${models[0][1].runs} runs`,
      confidence: Math.min(models[0][1].runs / 3, 1),
      type: 'quality',
    });
  }

  // Speed recommendation
  const fastest = [...models].sort((a, b) => a[1].avgDur - b[1].avgDur);
  if (fastest.length >= 2 && fastest[0][1].avgDur < fastest[1][1].avgDur * 0.7) {
    const speedup = ((1 - fastest[0][1].avgDur / fastest[1][1].avgDur) * 100).toFixed(0);
    recs.push({
      text: `${shortModel(fastest[0][0])} is ${speedup}% faster than ${shortModel(fastest[1][0])}`,
      confidence: Math.min(fastest[0][1].runs / 3, 1),
      type: 'speed',
    });
  }

  return recs;
}

/* ── Gate insights (from completed runs) ── */

interface GatePassRate {
  gate: string;
  passed: number;
  total: number;
  rate: number;
}

function computeGateInsights(history: BenchRun[]): GatePassRate[] {
  const gates = new Map<string, { passed: number; total: number }>();
  for (const run of history) {
    for (const result of run.results) {
      for (const v of result.gate_verdicts) {
        const prev = gates.get(v.gate) ?? { passed: 0, total: 0 };
        gates.set(v.gate, {
          passed: prev.passed + (v.passed ? 1 : 0),
          total: prev.total + 1,
        });
      }
    }
  }
  return [...gates.entries()]
    .map(([gate, { passed, total }]) => ({
      gate,
      passed,
      total,
      rate: total > 0 ? passed / total : 0,
    }))
    .sort((a, b) => a.gate.localeCompare(b.gate));
}

/* ── Component ── */

export default function BenchLearningInsights({ history, learningEvents, isRunning }: BenchLearningInsightsProps) {
  const { get } = useLiveApi();

  // Fetch learning data from API
  const [cascadeRouter, setCascadeRouter] = useState<CascadeRouterResponse | null>(null);
  const [efficiency, setEfficiency] = useState<EfficiencyResponse | null>(null);
  const [gateThresholds, setGateThresholds] = useState<GateThresholdsResponse | null>(null);

  useEffect(() => {
    (async () => {
      try {
        const data = await get<CascadeRouterResponse>('/api/learn/cascade');
        if (data) setCascadeRouter(data);
      } catch { /* server may be offline */ }
    })();
    (async () => {
      try {
        const data = await get<EfficiencyResponse>('/api/learn/efficiency');
        if (data) setEfficiency(data);
      } catch { /* ok */ }
    })();
    (async () => {
      try {
        const data = await get<GateThresholdsResponse>('/api/learning/gate-thresholds');
        if (data) setGateThresholds(data);
      } catch { /* ok */ }
    })();
  }, [get]);

  const recommendations = useMemo(() => computeRecommendations(history), [history]);
  const gateInsights = useMemo(() => computeGateInsights(history), [history]);
  const modelStats = cascadeRouter?.models ?? cascadeRouter?.model_stats ?? [];

  // Hero stats
  const totalObservations = cascadeRouter?.observations ?? 0;
  const totalModels = modelStats.length;
  const avgGatePass = gateInsights.length > 0
    ? gateInsights.reduce((s, g) => s + g.rate, 0) / gateInsights.length
    : 0;

  return (
    <div className="bench-learning">
      {/* Hero mosaic */}
      <div className="bench-learning-hero">
        <Mosaic columns={4}>
          <MosaicCell
            label="ROUTER OBSERVATIONS"
            value={totalObservations > 0 ? totalObservations.toLocaleString() : '-'}
            color="bone"
          />
          <MosaicCell
            label="TRACKED MODELS"
            value={totalModels > 0 ? String(totalModels) : '-'}
            color="rose"
          />
          <MosaicCell
            label="AVG GATE PASS"
            value={avgGatePass > 0 ? `${(avgGatePass * 100).toFixed(0)}%` : '-'}
            color="success"
          />
          <MosaicCell
            label="RECOMMENDATIONS"
            value={String(recommendations.length)}
            color="dream"
          />
        </Mosaic>
      </div>

      {/* Active learning indicator */}
      {isRunning && (
        <div className="bench-learning-active">
          <span className="bench-learning-pulse" />
          <span>Learning from active run ({learningEvents.length} insights captured)</span>
        </div>
      )}

      {/* Cascade Router Model Stats */}
      <Pane title="CASCADE ROUTER">
        {modelStats.length === 0 ? (
          <p className="bench-empty-text">No router data yet. Run benchmarks to populate.</p>
        ) : (
          <div className="task-table-wrap">
            <table className="task-table">
              <thead>
                <tr>
                  <th>Model</th>
                  <th>Provider</th>
                  <th>Trials</th>
                  <th>Pass Rate</th>
                  <th>Avg Cost</th>
                </tr>
              </thead>
              <tbody>
                {modelStats.map((m) => (
                  <tr key={m.model}>
                    <td className="mono">{shortModel(m.model)}</td>
                    <td className="mono">{m.provider ?? '-'}</td>
                    <td className="mono">{m.trials}</td>
                    <td className="mono">
                      {m.trials > 0
                        ? `${((m.successes / m.trials) * 100).toFixed(1)}%`
                        : '-'}
                    </td>
                    <td className="mono">
                      {m.avg_cost != null ? `$${m.avg_cost.toFixed(4)}` : '-'}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        )}
      </Pane>

      {/* Recommendations */}
      {recommendations.length > 0 && (
        <Pane title="RECOMMENDATIONS">
          <div className="bench-learning-recs">
            {recommendations.map((rec, i) => (
              <div key={i} className={`bench-learning-rec bench-learning-rec--${rec.type}`}>
                <span className="bench-learning-rec-icon">
                  {rec.type === 'cost' ? '$' : rec.type === 'quality' ? '*' : '>'}
                </span>
                <span className="bench-learning-rec-text">{rec.text}</span>
                <span className="bench-learning-rec-conf">
                  {(rec.confidence * 100).toFixed(0)}%
                </span>
              </div>
            ))}
          </div>
        </Pane>
      )}

      {/* Gate Insights */}
      <Pane title="GATE PASS RATES">
        {gateInsights.length === 0 ? (
          <p className="bench-empty-text">No gate data yet.</p>
        ) : (
          <div className="bench-learning-gates">
            {gateInsights.map((g) => (
              <div key={g.gate} className="bench-learning-gate-row">
                <span className="bench-learning-gate-name">{g.gate}</span>
                <div className="bench-learning-gate-bar">
                  <div
                    className="bench-learning-gate-fill"
                    style={{
                      width: `${g.rate * 100}%`,
                      background: g.rate >= 0.8 ? 'var(--success)' : g.rate >= 0.5 ? 'var(--warning)' : 'var(--rose-bright)',
                    }}
                  />
                </div>
                <span className="bench-learning-gate-pct">{(g.rate * 100).toFixed(0)}%</span>
                <span className="bench-learning-gate-count">{g.passed}/{g.total}</span>
              </div>
            ))}
          </div>
        )}
      </Pane>

      {/* Efficiency */}
      {efficiency && (efficiency.total_events ?? 0) > 0 && (
        <Pane title="EFFICIENCY METRICS">
          <div className="bench-learning-efficiency">
            <div className="param-row">
              <span className="param-label">Events</span>
              <span className="param-value mono">{efficiency.total_events?.toLocaleString()}</span>
            </div>
            {efficiency.avg_tokens_per_turn != null && (
              <div className="param-row">
                <span className="param-label">Avg Tokens/Turn</span>
                <span className="param-value mono">{efficiency.avg_tokens_per_turn.toLocaleString()}</span>
              </div>
            )}
            {efficiency.avg_cost_per_turn != null && (
              <div className="param-row">
                <span className="param-label">Avg Cost/Turn</span>
                <span className="param-value mono">${efficiency.avg_cost_per_turn.toFixed(4)}</span>
              </div>
            )}
          </div>
        </Pane>
      )}

      {/* Adaptive Gate Thresholds */}
      {gateThresholds && (gateThresholds.thresholds?.length ?? 0) > 0 && (
        <Pane title="ADAPTIVE GATE THRESHOLDS">
          <div className="task-table-wrap">
            <table className="task-table">
              <thead>
                <tr>
                  <th>Gate</th>
                  <th>Threshold</th>
                  <th>Observations</th>
                </tr>
              </thead>
              <tbody>
                {gateThresholds.thresholds!.map((t) => (
                  <tr key={t.gate}>
                    <td className="mono">{t.gate}</td>
                    <td className="mono">{(t.threshold * 100).toFixed(1)}%</td>
                    <td className="mono">{t.observations}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          </div>
        </Pane>
      )}

      {/* Live Learning Events Stream */}
      {learningEvents.length > 0 && (
        <Pane title="LEARNING EVENTS">
          <div className="bench-learning-events">
            {learningEvents.map((evt, i) => (
              <div key={i} className="bench-learning-event">
                <span className="bench-learning-event-text">{evt.insight}</span>
                {evt.metric && (
                  <span className="bench-learning-event-metric">
                    {evt.metric}
                    {evt.before != null && evt.after != null && (
                      <> ({evt.before.toFixed(2)} &rarr; {evt.after.toFixed(2)})</>
                    )}
                  </span>
                )}
                {evt.confidence != null && (
                  <span className="bench-learning-event-conf">
                    {(evt.confidence * 100).toFixed(0)}%
                  </span>
                )}
              </div>
            ))}
          </div>
        </Pane>
      )}
    </div>
  );
}
