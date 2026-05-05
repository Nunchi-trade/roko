// --- src/lib/scenario-runners/index.ts ---
import type { ClickableScenario } from '../scenarios';

import { pipelineScenario } from './pipeline';
import { prdPipeline } from './prd-pipeline';
import { prdResearchLoop } from './prd-research-loop';
import { race } from './race';
import { gateRetry } from './gate-retry';
import { providers } from './providers';
import { providerRace } from './provider-race';
import { explore } from './explore';
import { knowledgeAccumulation } from './knowledge-accumulation';
import { dreamConsolidation } from './dream-consolidation';
import { chat } from './chat';
import { knowledgeTransfer } from './knowledge-transfer';
import { chainIntelligence } from './chain-intelligence';
import { mirage } from './mirage';
import { isfrAgents } from './isfr-agents';

export {
  pipelineScenario,
  prdPipeline, prdResearchLoop, race, gateRetry, providers, providerRace,
  explore, knowledgeAccumulation, dreamConsolidation, chat, knowledgeTransfer,
  chainIntelligence, mirage, isfrAgents,
};

export const allScenarios: ClickableScenario[] = [
  pipelineScenario,
  prdPipeline, prdResearchLoop, race, gateRetry, providers, providerRace,
  explore, knowledgeAccumulation, dreamConsolidation, chat, knowledgeTransfer,
  chainIntelligence, mirage, isfrAgents,
];
