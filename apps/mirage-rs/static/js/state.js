/* ================================================================
   SHARED STATE — single mutable object imported by all modules
   ================================================================ */

import { DEFAULT_REMOTE_BASE, mergeAgentSources, transportOptionsForAgent } from './agent_registry.js';

function createRegistryState() {
  return {
    identityAgents: [],
    relayAgents: [],
    mergedAgents: [],
    identityRegistryAddress: null,
    selectedAgentKey: '',
    transportPreference: 'auto',
    messageResult: null,
    lastUpdatedAt: 0,
    errors: { identity: null, relay: null },
  };
}

export var state = {
  remoteBase: DEFAULT_REMOTE_BASE,
  rpcUrl: DEFAULT_REMOTE_BASE,
  apiBase: DEFAULT_REMOTE_BASE + '/api',
  relayBase: DEFAULT_REMOTE_BASE + '/relay',
  wsUrl: null,
  connected: false,
  wsLive: false,
  ws: null,
  forkBlock: 0,
  // chain data
  blocks: [],          // ObservedBlock[]: {number, hash, timestamp, gasUsed, gasLimit, baseFeeGwei, txCount, saturation, fresh}
  insights: new Map(), // id -> {id, kind, content, author, conf, chall, weight, createdAt, similarity, score, state}
  pheromones: [],      // particle[]: {kind, content, intensity, x, y, anchorX, anchorY, vx, vy, age, deposited, halfLife, pulse, chainId, decayProjection}
  selectedBlock: null,
  selectedNode: null,
  hoveredNode: null,
  // counts
  confirmsCount: 0,
  challengesCount: 0,
  observedConfirms: 0,
  observedChallenges: 0,
  // topology
  topoNodes: [],
  topoEdges: [],
  // heatmap
  heatmapBuckets: [],
  // kinds
  kindsData: null,
  // agent log
  agentLog: [],
  seenAuthors: new Set(),
  // request log
  requestLog: [],
  // growth timeline
  growthSeries: [],
  // chain stats
  chainInsightsTotal: null,
  chainPheromonesTotal: null,
  insightsTotalPrev: 0,
  // posts tracker
  postsLastMin: [],
  // registered agents
  registeredAgents: [],
  registry: createRegistryState(),
  // sparkline series
  series: { block:[], fee:[], sat:[], insights:[], phero:[], agents:[], cache:[], rpc:[], search:[], posts:[] },
  // rpc counters
  rpc: { total: 0, prev: 0, errors: 0 },
  // pollers
  pollers: {},
};

export function applyAgentDiscovery(discovery) {
  state.registry.identityAgents = discovery.identityAgents || [];
  state.registry.relayAgents = discovery.relayAgents || [];
  state.registry.identityRegistryAddress = discovery.identityRegistryAddress || null;
  state.registry.errors = discovery.errors || { identity: null, relay: null };
  state.registry.lastUpdatedAt = Date.now();
  state.registry.mergedAgents = mergeAgentSources(
    state.registry.identityAgents,
    state.registry.relayAgents
  );
  state.registeredAgents = state.registry.mergedAgents.slice();

  if (!state.registry.mergedAgents.length) {
    state.registry.selectedAgentKey = '';
    return state.registry.mergedAgents;
  }

  if (!getSelectedMergedAgent()) {
    state.registry.selectedAgentKey = state.registry.mergedAgents[0].key;
  }

  var selected = getSelectedMergedAgent();
  var options = listTransportOptionsForAgent(selected);
  var stillValid = options.some(function(option) {
    return option.value === state.registry.transportPreference;
  });
  if (!stillValid) {
    state.registry.transportPreference = 'auto';
  }

  return state.registry.mergedAgents;
}

export function resetRegistryState() {
  state.registry = createRegistryState();
  state.registeredAgents = [];
}

export function getSelectedMergedAgent() {
  var key = state.registry.selectedAgentKey;
  if (!key) return null;
  for (var i = 0; i < state.registry.mergedAgents.length; i++) {
    if (state.registry.mergedAgents[i].key === key) {
      return state.registry.mergedAgents[i];
    }
  }
  return null;
}

export function setSelectedMergedAgent(agentKey) {
  state.registry.selectedAgentKey = agentKey || '';
}

export function setRegistryTransportPreference(preference) {
  state.registry.transportPreference = preference || 'auto';
}

export function setRegistryMessageResult(result) {
  state.registry.messageResult = result || null;
}

export function listTransportOptionsForAgent(agent) {
  return transportOptionsForAgent(agent, state.remoteBase);
}
