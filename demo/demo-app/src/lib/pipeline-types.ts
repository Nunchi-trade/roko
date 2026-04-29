/**
 * Pipeline state machine types for the demo app.
 *
 * Models the lifecycle of a scenario run through discrete stages,
 * with a reducer for predictable state transitions.
 */

// ── Stage ────────────────────────────────────────────────────

export type PipelineStage =
  | 'idle'           // no scenario active
  | 'selecting'      // browsing scenarios
  | 'configuring'    // setting up scenario params
  | 'starting'       // workspace creation / terminal attach
  | 'prd_generating' // PRD agent running
  | 'planning'       // plan generation
  | 'executing'      // agents running tasks
  | 'gate_checking'  // gates validating
  | 'paused'         // user paused
  | 'failed'         // pipeline error
  | 'complete';      // all tasks done

// ── Gate result ──────────────────────────────────────────────

export interface GateResult {
  gate: string;
  passed: boolean;
  message: string;
  durationMs: number;
}

// ── State ────────────────────────────────────────────────────

export interface PipelineState {
  stage: PipelineStage;
  scenarioId: string | null;
  scenarioTitle: string | null;
  activeTaskId: string | null;
  activeTaskTitle: string | null;
  gateResults: GateResult[];
  startedAt: number | null;
  elapsed: number;
  error: string | null;
  completedTasks: number;
  totalTasks: number;
}

// ── Actions ──────────────────────────────────────────────────

export type PipelineAction =
  | { type: 'SELECT_SCENARIO'; scenarioId: string; title: string }
  | { type: 'START' }
  | { type: 'SET_STAGE'; stage: PipelineStage }
  | { type: 'TASK_STARTED'; taskId: string; title: string; total: number }
  | { type: 'TASK_COMPLETED'; taskId: string }
  | { type: 'GATE_RESULT'; result: GateResult }
  | { type: 'PAUSE' }
  | { type: 'RESUME' }
  | { type: 'FAIL'; error: string }
  | { type: 'COMPLETE' }
  | { type: 'RESET' }
  | { type: 'TICK'; elapsed: number };

// ── Initial state ────────────────────────────────────────────

export const INITIAL_PIPELINE_STATE: PipelineState = {
  stage: 'idle',
  scenarioId: null,
  scenarioTitle: null,
  activeTaskId: null,
  activeTaskTitle: null,
  gateResults: [],
  startedAt: null,
  elapsed: 0,
  error: null,
  completedTasks: 0,
  totalTasks: 0,
};

// ── Reducer ──────────────────────────────────────────────────

/** Stages from which the pipeline can be paused. */
const PAUSABLE: ReadonlySet<PipelineStage> = new Set([
  'prd_generating',
  'planning',
  'executing',
  'gate_checking',
]);

/** The stage the pipeline was in before being paused, so RESUME can restore it. */
let _stageBeforePause: PipelineStage = 'executing';

export function pipelineReducer(
  state: PipelineState,
  action: PipelineAction,
): PipelineState {
  switch (action.type) {
    case 'SELECT_SCENARIO': {
      if (state.stage !== 'idle' && state.stage !== 'selecting') return state;
      return {
        ...state,
        stage: 'selecting',
        scenarioId: action.scenarioId,
        scenarioTitle: action.title,
      };
    }

    case 'START': {
      if (state.stage !== 'selecting' && state.stage !== 'configuring') return state;
      return {
        ...state,
        stage: 'starting',
        startedAt: Date.now(),
        elapsed: 0,
        gateResults: [],
        completedTasks: 0,
        totalTasks: 0,
        error: null,
      };
    }

    case 'SET_STAGE': {
      // Allow setting stage from any non-terminal state, or re-entering the same stage.
      if (state.stage === 'complete' || state.stage === 'failed') return state;
      return { ...state, stage: action.stage };
    }

    case 'TASK_STARTED': {
      if (state.stage !== 'executing' && state.stage !== 'starting' && state.stage !== 'planning') {
        return state;
      }
      return {
        ...state,
        stage: 'executing',
        activeTaskId: action.taskId,
        activeTaskTitle: action.title,
        totalTasks: action.total,
      };
    }

    case 'TASK_COMPLETED': {
      if (state.stage !== 'executing') return state;
      return {
        ...state,
        activeTaskId: null,
        activeTaskTitle: null,
        completedTasks: state.completedTasks + 1,
      };
    }

    case 'GATE_RESULT': {
      if (state.stage !== 'executing' && state.stage !== 'gate_checking') return state;
      return {
        ...state,
        stage: 'gate_checking',
        gateResults: [...state.gateResults, action.result],
      };
    }

    case 'PAUSE': {
      if (!PAUSABLE.has(state.stage)) return state;
      _stageBeforePause = state.stage;
      return { ...state, stage: 'paused' };
    }

    case 'RESUME': {
      if (state.stage !== 'paused') return state;
      return { ...state, stage: _stageBeforePause };
    }

    case 'FAIL': {
      // Can fail from any active stage.
      if (state.stage === 'idle' || state.stage === 'complete') return state;
      return { ...state, stage: 'failed', error: action.error };
    }

    case 'COMPLETE': {
      if (state.stage === 'idle' || state.stage === 'failed') return state;
      return {
        ...state,
        stage: 'complete',
        activeTaskId: null,
        activeTaskTitle: null,
      };
    }

    case 'RESET': {
      return { ...INITIAL_PIPELINE_STATE };
    }

    case 'TICK': {
      if (state.startedAt === null) return state;
      return { ...state, elapsed: action.elapsed };
    }

    default:
      return state;
  }
}
