/**
 * Demo scenarios — type definitions and re-exports.
 *
 * Individual scenario implementations live in ./scenario-runners/.
 * Helper functions live in ./scenario-helpers.ts.
 */
import type { TerminalHandle } from '../hooks/useTerminal';
import type { PlaybackController, TimelineStepper } from './playback-controller';
import type {
  PipelineDemoState,
  PipelineEvent,
  PipelineScenarioExample,
  PipelineStreamState,
  PipelineTask,
} from './prd-pipeline-types';
import type { AgentIdentity } from '../components/Spectre/AgentIdentity';

// ── Types ───────────────────────────────────────────────────

export interface ScenarioStep {
  label: string;
  sublabel?: string;
}

export interface ScenarioContext {
  entries: TerminalHandle[];
  playback: PlaybackController;
  timeline: TimelineStepper;
  setMetric: (id: string, value: string) => void;
  setGate: (name: string, status: 'pass' | 'fail' | 'pending') => void;
  logCommand: (cmd: string, desc: string) => void;
  logCommandComplete: (cmd: string, ok: boolean) => void;
  setPipeline: (state: PipelineDemoState) => void;
  patchPipeline: (patch: Partial<PipelineDemoState>) => void;
  patchPipelineStream: (patch: Partial<PipelineStreamState>) => void;
  updatePipelineTask: (planId: string, taskId: string, patch: Partial<PipelineTask>) => void;
  appendPipelineEvent: (event: PipelineEvent) => void;
  pipelineExample: PipelineScenarioExample;
  /** Active model from ConfigWidget (e.g. "glm51", "sonnet") */
  activeModel?: string;
  paused: { current: boolean };
  running: { current: boolean };
  /** Server-created workspace directory path (pre-created before scenario runs). */
  workspaceDir: string;
  /** Create an additional workspace for multi-workspace scenarios. */
  createWorkspace: (prefix: string) => Promise<string>;
}

export interface Scenario {
  id: string;
  title: string;
  subtitle: string;
  panes: 1 | 2 | 4;
  labels: string[];
  panel: boolean;
  promptBar: boolean;
  mirageBar?: boolean;
  steps: ScenarioStep[];
  /** Category tag for visual grouping */
  category: 'pipeline' | 'comparison' | 'exploration' | 'learning' | 'chain';
  /** Key features to highlight in preview */
  features: string[];
  /** Estimated duration hint */
  durationHint: string;
  /** Accent color for this scenario's preview */
  accent: 'rose' | 'teal' | 'amber' | 'violet' | 'emerald';
  /** Icon identifier for the scenario diagram */
  icon: 'pipeline' | 'race' | 'gate' | 'grid' | 'explore' | 'knowledge' | 'dream' | 'chat' | 'transfer' | 'chain' | 'evm';
  /** Optional per-pane agent identities for multi-agent terminal split.
   *  Length should match `panes`; each entry maps to the corresponding terminal. */
  agents?: AgentIdentity[];
  run(ctx: ScenarioContext): Promise<void>;
}

// ── Re-export ────────────────────────────────────────────────

export { allScenarios as SCENARIOS } from './scenario-runners';
