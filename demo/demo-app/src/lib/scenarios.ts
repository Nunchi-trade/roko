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

export interface PipelineContext {
  setPipeline: (state: PipelineDemoState) => void;
  patchPipeline: (patch: Partial<PipelineDemoState>) => void;
  patchPipelineStream: (patch: Partial<PipelineStreamState>) => void;
  updatePipelineTask: (planId: string, taskId: string, patch: Partial<PipelineTask>) => void;
  appendPipelineEvent: (event: PipelineEvent) => void;
  example: PipelineScenarioExample;
}

export interface ScenarioContext {
  entries: TerminalHandle[];
  playback: PlaybackController;
  timeline: TimelineStepper;
  setMetric: (id: string, value: string) => void;
  setGate: (name: string, status: 'pass' | 'fail' | 'pending') => void;
  logCommand: (cmd: string, desc: string) => void;
  logCommandComplete: (cmd: string, ok: boolean) => void;
  signal: AbortSignal;
  pipeline?: PipelineContext;
  /** @deprecated Use ctx.pipeline!.setPipeline() */
  setPipeline: (state: PipelineDemoState) => void;
  /** @deprecated Use ctx.pipeline!.patchPipeline() */
  patchPipeline: (patch: Partial<PipelineDemoState>) => void;
  /** @deprecated Use ctx.pipeline!.patchPipelineStream() */
  patchPipelineStream: (patch: Partial<PipelineStreamState>) => void;
  /** @deprecated Use ctx.pipeline!.updatePipelineTask() */
  updatePipelineTask: (planId: string, taskId: string, patch: Partial<PipelineTask>) => void;
  /** @deprecated Use ctx.pipeline!.appendPipelineEvent() */
  appendPipelineEvent: (event: PipelineEvent) => void;
  /** @deprecated Use ctx.pipeline!.example */
  pipelineExample: PipelineScenarioExample;
  /** Active model from ConfigWidget (e.g. "glm51", "sonnet") */
  activeModel?: string;
  /** @deprecated Use signal.aborted */
  paused: { current: boolean };
  /** @deprecated Use signal.aborted */
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
  panes: 1 | 2 | 4 | 8;
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

// ── ClickableScenario ────────────────────────────────────────

export interface CommandDef {
  id: string;
  command: string;
  description: string;
  timeout?: number;
}

export interface ClickableScenario extends Omit<Scenario, 'run'> {
  commands: CommandDef[];
  runCommand(ctx: ScenarioContext, commandId: string): Promise<{ ok: boolean; error?: string }>;
}

export function isClickableScenario(s: Scenario | ClickableScenario): s is ClickableScenario {
  return 'commands' in s && 'runCommand' in s;
}

// ── Re-export ────────────────────────────────────────────────

export { allScenarios as SCENARIOS } from './scenario-runners';
