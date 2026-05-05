// --- src/lib/scenario-runners/archive/index.ts ---
import type { ClickableScenario } from '../../scenarios';

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

export { PRD_IDEA, PRD_PIPELINE_COMMANDS, getRuntimeCmd, prdPipeline } from './prd-pipeline';
export { prdResearchLoop, RESEARCH_LOOP_COMMANDS } from './prd-research-loop';
export { race, RACE_COMMANDS } from './race';
export { gateRetry, GATE_RETRY_COMMANDS } from './gate-retry';
export { providers, PROVIDERS_COMMANDS } from './providers';
export { providerRace, PROVIDER_RACE_COMMANDS } from './provider-race';
export { explore, EXPLORE_COMMANDS } from './explore';
export { knowledgeAccumulation, KNOWLEDGE_ACCUMULATION_COMMANDS } from './knowledge-accumulation';
export { dreamConsolidation, DREAM_COMMANDS } from './dream-consolidation';
export { chat, CHAT_COMMANDS } from './chat';
export { knowledgeTransfer, KNOWLEDGE_TRANSFER_COMMANDS } from './knowledge-transfer';
export { chainIntelligence, CHAIN_INTELLIGENCE_COMMANDS } from './chain-intelligence';
export { mirage, MIRAGE_COMMANDS } from './mirage';
export { isfrAgents, ISFR_COMMANDS } from './isfr-agents';

export const archivedScenarios: ClickableScenario[] = [
  prdPipeline,
  prdResearchLoop,
  race,
  gateRetry,
  providers,
  providerRace,
  explore,
  knowledgeAccumulation,
  dreamConsolidation,
  chat,
  knowledgeTransfer,
  chainIntelligence,
  mirage,
  isfrAgents,
];
