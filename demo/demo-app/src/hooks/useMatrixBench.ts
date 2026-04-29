import { useState, useCallback, useEffect } from 'react';
import { useLiveApi } from './useLiveApi';
import { useBenchSSE } from './useBenchSSE';
import type {
  AgentStrategy,
  BenchModel,
  BenchRunConfig,
  BenchTaskResult,
  ConfigPreset,
  MatrixLaneSummary,
  BenchSSEEvent,
} from '../lib/bench-types';

/* ── Default presets (mirror STRATEGIES in Bench.tsx) ── */

export const DEFAULT_PRESETS: ConfigPreset[] = [
  { id: 'minimal', label: 'Minimal', strategy: 'minimal', description: 'Basic agent, no enrichment' },
  { id: 'context_enriched', label: 'Context-Enriched', strategy: 'context_enriched', description: 'With context bidders' },
  { id: 'neuro_augmented', label: 'Neuro-Augmented', strategy: 'neuro_augmented', description: 'With knowledge store' },
  { id: 'full_cascade', label: 'Full Cascade', strategy: 'full_cascade', description: 'Complete pipeline with replan' },
];

/* ── Cell state for the matrix grid ── */

export type MatrixCellStatus = 'idle' | 'running' | 'pass' | 'fail' | 'partial';

export interface MatrixCell {
  modelId: string;
  presetId: string;
  laneId?: string;
  status: MatrixCellStatus;
  passRate?: number;
  costUsd?: number;
  results: BenchTaskResult[];
}

export type MatrixStatus = 'idle' | 'running' | 'completed' | 'cancelled' | 'partial_failure';

/* ── Hook ── */

export function useMatrixBench(models: BenchModel[]) {
  const { post } = useLiveApi();

  // Selected models (Y-axis rows)
  const [selectedModels, setSelectedModels] = useState<string[]>([]);

  // Presets (X-axis columns)
  const [presets, setPresets] = useState<ConfigPreset[]>(DEFAULT_PRESETS);

  // Matrix run state
  const [matrixId, setMatrixId] = useState<string | null>(null);
  const [status, setStatus] = useState<MatrixStatus>('idle');
  const [cells, setCells] = useState<MatrixCell[][]>([]);

  // Lane-id to (row, col) mapping
  const [laneMap, setLaneMap] = useState<Map<string, { row: number; col: number }>>(new Map());

  // Bench-id to lane-id mapping (each lane is a bench run)
  const [benchToLane, setBenchToLane] = useState<Map<string, string>>(new Map());

  // SSE: subscribe without benchId filter to capture matrix events
  const { lastEvent } = useBenchSSE({
    enabled: status === 'running',
  });

  // Toggle model selection
  const toggleModel = useCallback((modelId: string) => {
    setSelectedModels((prev) =>
      prev.includes(modelId)
        ? prev.filter((m) => m !== modelId)
        : [...prev, modelId],
    );
  }, []);

  // Toggle preset selection
  const togglePreset = useCallback((presetId: string) => {
    setPresets((prev) => {
      const exists = prev.find((p) => p.id === presetId);
      if (exists) {
        // Don't remove the last preset
        if (prev.length <= 1) return prev;
        return prev.filter((p) => p.id !== presetId);
      }
      const defaultPreset = DEFAULT_PRESETS.find((p) => p.id === presetId);
      if (defaultPreset) return [...prev, defaultPreset];
      return prev;
    });
  }, []);

  // Build the cell grid whenever models/presets change
  useEffect(() => {
    if (status === 'running') return; // Don't rebuild while running
    setCells(
      selectedModels.map((modelId) =>
        presets.map((preset) => ({
          modelId,
          presetId: preset.id,
          status: 'idle' as MatrixCellStatus,
          results: [],
        })),
      ),
    );
  }, [selectedModels, presets, status]);

  // Estimated cost for the matrix
  const estimatedTotalCost = useCallback(
    (_suiteTaskCount: number, suiteCostUsd: number): number => {
      const totalLanes = selectedModels.length * presets.length;
      return totalLanes * suiteCostUsd;
    },
    [selectedModels.length, presets.length],
  );

  // Launch matrix run
  const startMatrix = useCallback(
    async (suiteId: string, baseConfig: Partial<BenchRunConfig>) => {
      if (selectedModels.length === 0 || presets.length === 0) return;

      // Build lanes: models x presets
      const lanes: { model: string; backend?: string; strategy: AgentStrategy; label: string; overrides: Partial<BenchRunConfig> }[] = [];
      const newLaneMap = new Map<string, { row: number; col: number }>();

      for (let row = 0; row < selectedModels.length; row++) {
        const modelId = selectedModels[row];
        const modelInfo = models.find((m) => m.id === modelId);
        for (let col = 0; col < presets.length; col++) {
          const preset = presets[col];
          const laneLabel = `${modelId.split('-').slice(0, 2).join('-')} / ${preset.label}`;
          lanes.push({
            model: modelId,
            backend: modelInfo?.provider,
            strategy: preset.strategy,
            label: laneLabel,
            overrides: {
              ...baseConfig,
              model: modelId,
              provider: modelInfo?.provider,
              strategy: preset.strategy,
              temperature: preset.temperature ?? baseConfig.temperature,
              max_tokens: preset.maxTokens ?? baseConfig.max_tokens,
            },
          });
          // Lane ID will be assigned by server; use index as placeholder
          const placeholderId = `lane-${row}-${col}`;
          newLaneMap.set(placeholderId, { row, col });
        }
      }

      try {
        const res = await post<{ id?: string; matrix_id?: string; lane_ids: string[] }>(
          '/api/bench/matrix',
          { suite_id: suiteId, lanes },
        );
        const nextMatrixId = res.matrix_id ?? res.id;

        if (!nextMatrixId) return;

        // Map real lane IDs to grid positions
        const realLaneMap = new Map<string, { row: number; col: number }>();
        const realBenchToLane = new Map<string, string>();
        let idx = 0;
        for (let row = 0; row < selectedModels.length; row++) {
          for (let col = 0; col < presets.length; col++) {
            const laneId = res.lane_ids[idx];
            if (laneId) {
              realLaneMap.set(laneId, { row, col });
              // The lane_id doubles as the bench_id for per-lane events
              realBenchToLane.set(laneId, laneId);
            }
            idx++;
          }
        }

        setLaneMap(realLaneMap);
        setBenchToLane(realBenchToLane);
        setMatrixId(nextMatrixId);
        setStatus('running');

        // Initialize cells as running
        setCells(
          selectedModels.map((modelId, row) =>
            presets.map((preset, col) => ({
              modelId,
              presetId: preset.id,
              laneId: res.lane_ids[row * presets.length + col],
              status: 'running' as MatrixCellStatus,
              results: [],
            })),
          ),
        );
      } catch {
        // Server unavailable
      }
    },
    [selectedModels, presets, models, post],
  );

  // Process SSE events
  useEffect(() => {
    if (!lastEvent || status !== 'running') return;

    const evt = lastEvent as BenchSSEEvent;

    switch (evt.type) {
      case 'BenchTaskCompleted': {
        // Match bench_id to a lane
        const benchId = evt.bench_id;
        const laneId = benchToLane.get(benchId) ?? benchId;
        const pos = laneMap.get(laneId);
        if (!pos) break;

        setCells((prev) => {
          const next = prev.map((r) => [...r]);
          const cell = { ...next[pos.row][pos.col] };
          cell.results = [...cell.results, evt.result];
          cell.status = 'running';
          next[pos.row][pos.col] = cell;
          return next;
        });
        break;
      }

      case 'MatrixLaneCompleted': {
        const pos = laneMap.get(evt.lane_id);
        if (!pos) break;

        setCells((prev) => {
          const next = prev.map((r) => [...r]);
          const cell = { ...next[pos.row][pos.col] };
          cell.passRate = evt.pass_rate;
          cell.costUsd = evt.cost_usd;
          cell.status = evt.pass_rate >= 0.5 ? 'pass' : 'fail';
          next[pos.row][pos.col] = cell;
          return next;
        });
        break;
      }

      case 'MatrixRunCompleted': {
        if (evt.matrix_id !== matrixId) break;

        // Update all cells from summary
        setCells((prev) => {
          const next = prev.map((r) => [...r]);
          for (const lane of evt.summary as MatrixLaneSummary[]) {
            const pos = laneMap.get(lane.lane_id);
            if (!pos) continue;
            const cell = { ...next[pos.row][pos.col] };
            cell.passRate = lane.pass_rate;
            cell.costUsd = lane.cost_usd;
            cell.status = lane.pass_rate >= 0.5 ? 'pass' : 'fail';
            next[pos.row][pos.col] = cell;
          }
          return next;
        });
        setStatus('completed');
        break;
      }
    }
  }, [lastEvent, status, laneMap, benchToLane, matrixId]);

  // Total lanes count
  const totalLanes = selectedModels.length * presets.length;

  return {
    // Selection
    selectedModels,
    setSelectedModels,
    toggleModel,
    presets,
    setPresets,
    togglePreset,

    // State
    matrixId,
    status,
    cells,
    totalLanes,

    // Actions
    startMatrix,
    estimatedTotalCost,
  };
}
