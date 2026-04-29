/**
 * Data layer barrel export.
 *
 * Re-exports from app/DataHub (the canonical store) and data/types
 * so consumers can import from either `../data` or `../app`.
 */

// Store hook
export { useDataHub } from '../app/DataHub';

// Store interface + slice types
export type {
  DataHub,
  ServerStatus,
  StreamStatus,
  WorkspaceInfo,
  AgentInfo,
  EpisodeInfo,
  InferenceRecord,
} from '../app/DataHub';

// Standalone domain types
export type {
  HealthSnapshot,
  RokoConfig,
} from './types';
