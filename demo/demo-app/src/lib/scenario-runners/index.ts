// --- src/lib/scenario-runners/index.ts ---
import type { ClickableScenario } from '../scenarios';

import { costScenario } from './cost';
import { pipelineScenario } from './pipeline';
import { memoryScenario } from './memory';
import { isfrScenario } from './isfr';
import { oracleScenario } from './oracle';

export {
  costScenario,
  pipelineScenario,
  memoryScenario,
  isfrScenario,
  oracleScenario,
};

export const allScenarios: ClickableScenario[] = [
  costScenario,
  pipelineScenario,
  memoryScenario,
  isfrScenario,
  oracleScenario,
];
