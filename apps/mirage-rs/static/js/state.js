/* ================================================================
   SHARED STATE — single mutable object imported by all modules
   ================================================================ */

export var state = {
  rpcUrl: 'http://127.0.0.1:8545',
  wsUrl: null,
  connected: false,
  wsLive: false,
  ws: null,
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
  // sparkline series
  series: { block:[], fee:[], sat:[], insights:[], phero:[], agents:[], cache:[], rpc:[], search:[], posts:[] },
  // rpc counters
  rpc: { total: 0, prev: 0, errors: 0 },
  // pollers
  pollers: {},
};
