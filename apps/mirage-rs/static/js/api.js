/* ================================================================
   API — base normalization, JSON-RPC, REST, relay, identity reads
   ================================================================ */

import { state } from './state.js';
import {
  IDENTITY_REGISTRY_ALIAS,
  IDENTITY_REGISTRY_CANONICAL,
  OWNER_OF_SELECTOR,
  PASSPORT_MINTED_TOPIC,
  TOKEN_URI_SELECTOR,
  buildCallData,
  decodeAddressResult,
  decodeCardDataUri,
  decodeStringResult,
  deriveRemoteBases,
  isEmptyCode,
  joinUrl,
  normalizeCardEndpoint,
  normalizeRemoteBase,
  parseTopicUint,
  selectTransportPath,
} from './agent_registry.js';

var REMOTE_BASE_KEY = 'mirage.dashboard.remoteBase';
var rpcId = 1;
var cardCache = new Map();

/* ---------- Base URL wiring ---------- */
export function loadInitialRemoteBase() {
  if (typeof window === 'undefined') return state.remoteBase;
  var params = new URLSearchParams(window.location.search);
  var fromQuery = params.get('base') || params.get('mirage');
  if (fromQuery) return normalizeRemoteBase(fromQuery);
  try {
    var saved = window.localStorage.getItem(REMOTE_BASE_KEY);
    if (saved) return normalizeRemoteBase(saved);
  } catch (e) { /* ignore */ }
  return normalizeRemoteBase(state.remoteBase);
}

export function setRemoteBase(nextBase) {
  var bases = deriveRemoteBases(nextBase || state.remoteBase);
  state.remoteBase = bases.remoteBase;
  state.rpcUrl = bases.rpcUrl;
  state.apiBase = bases.apiBase;
  state.relayBase = bases.relayBase;
  state.wsUrl = bases.wsUrl;
  cardCache.clear();
  if (typeof window !== 'undefined') {
    try {
      window.localStorage.setItem(REMOTE_BASE_KEY, bases.remoteBase);
    } catch (e) { /* ignore */ }
  }
  return bases;
}

/* ---------- Generic requests ---------- */
async function requestJson(url, opts) {
  var method = opts && opts.method ? opts.method : 'GET';
  var label = opts && opts.label ? opts.label : 'api';
  var message = opts && opts.message ? opts.message : method + ' ' + url;
  state.rpc.total++;
  logReq(label, message);
  var t0 = performance.now();
  try {
    var resp = await fetch(url, {
      method: method,
      headers: (opts && opts.headers) || undefined,
      body: opts && opts.body ? opts.body : undefined,
    });
    var text = await resp.text();
    if (!resp.ok) {
      throw new Error(text || ('HTTP ' + resp.status));
    }
    var json = text ? JSON.parse(text) : null;
    var ms = performance.now() - t0;
    logReq('ok', message + ' ' + Math.round(ms) + 'ms');
    return { data: json, ms: ms };
  } catch (e) {
    if (opts && opts.softFail) {
      logReq('warn', message + ' unavailable: ' + e.message);
      return null;
    }
    state.rpc.errors++;
    logReq('err', message + ' FAILED: ' + e.message);
    throw e;
  }
}

/* ---------- JSON-RPC ---------- */
export async function rpc(method, params) {
  var id = rpcId++;
  var result = await requestJson(state.rpcUrl, {
    method: 'POST',
    label: 'rpc',
    message: method + ' #' + id,
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ jsonrpc: '2.0', method: method, params: params || [], id: id }),
  });
  return { result: result.data ? result.data.result : null, error: result.data ? result.data.error : null, ms: result.ms };
}

async function ethCall(to, data) {
  var response = await rpc('eth_call', [{ to: to, data: data }, 'latest']);
  if (response.error) {
    throw new Error(response.error.message || 'eth_call failed');
  }
  return response.result;
}

/* ---------- REST ---------- */
export async function api(path) {
  return requestJson(joinUrl(state.apiBase, path), {
    label: 'api',
    message: 'GET /api' + path,
  });
}

export async function apiPost(path, body) {
  return requestJson(joinUrl(state.apiBase, path), {
    method: 'POST',
    label: 'api',
    message: 'POST /api' + path,
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
}

export async function relayApi(path, softFail) {
  return requestJson(joinUrl(state.relayBase, path), {
    label: 'relay',
    message: 'GET /relay' + path,
    softFail: !!softFail,
  });
}

export async function relayPost(path, body) {
  return requestJson(joinUrl(state.relayBase, path), {
    method: 'POST',
    label: 'relay',
    message: 'POST /relay' + path,
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  });
}

/* ---------- Agent discovery ---------- */
export async function discoverAgentRegistry() {
  var identityError = null;
  var relayError = null;
  var identityRegistryAddress = null;
  var identityAgents = [];
  var relayAgents = [];

  try {
    var identity = await discoverIdentityAgents();
    identityRegistryAddress = identity.identityRegistryAddress;
    identityAgents = identity.agents;
  } catch (e) {
    identityError = e.message;
  }

  try {
    relayAgents = await discoverRelayAgents();
  } catch (e2) {
    relayError = e2.message;
  }

  return {
    identityRegistryAddress: identityRegistryAddress,
    identityAgents: identityAgents,
    relayAgents: relayAgents,
    errors: {
      identity: identityError,
      relay: relayError,
    },
  };
}

async function discoverIdentityAgents() {
  var identityRegistryAddress = await resolveIdentityRegistryAddress();
  if (!identityRegistryAddress) {
    return { identityRegistryAddress: null, agents: [] };
  }

  var latest = await rpc('eth_blockNumber', []);
  if (latest.error) {
    throw new Error(latest.error.message || 'eth_blockNumber failed');
  }
  var latestBlock = parseInt(latest.result || '0x0', 16);
  var fromBlock = state.forkBlock && latestBlock >= state.forkBlock
    ? state.forkBlock
    : Math.max(0, latestBlock - 10000);
  var logs = await rpc('eth_getLogs', [{
    address: identityRegistryAddress,
    fromBlock: '0x' + fromBlock.toString(16),
    toBlock: 'latest',
    topics: [PASSPORT_MINTED_TOPIC],
  }]);
  if (logs.error) {
    throw new Error(logs.error.message || 'eth_getLogs failed');
  }
  var entries = Array.isArray(logs.result) ? logs.result : [];
  var passportIds = [];
  var seen = new Set();
  for (var i = 0; i < entries.length; i++) {
    var topic = entries[i] && entries[i].topics ? entries[i].topics[1] : null;
    var passportId = parseTopicUint(topic);
    if (!passportId || seen.has(passportId)) continue;
    seen.add(passportId);
    passportIds.push(passportId);
  }

  var agents = [];
  for (var j = 0; j < passportIds.length; j++) {
    var agent = await readIdentityPassport(identityRegistryAddress, passportIds[j]);
    if (agent) agents.push(agent);
  }
  return { identityRegistryAddress: identityRegistryAddress, agents: agents };
}

async function readIdentityPassport(identityRegistryAddress, passportId) {
  try {
    var ownerHex = await ethCall(identityRegistryAddress, buildCallData(OWNER_OF_SELECTOR, passportId));
    var owner = decodeAddressResult(ownerHex);
    if (!owner || owner === '0x0000000000000000000000000000000000000000') return null;

    var cardUri = '';
    try {
      var cardUriHex = await ethCall(identityRegistryAddress, buildCallData(TOKEN_URI_SELECTOR, passportId));
      cardUri = decodeStringResult(cardUriHex);
    } catch (e) {
      cardUri = '';
    }

    var card = await resolveAgentCard(cardUri);
    var agentId = card && card.name ? card.name : ('passport-' + passportId);
    return {
      key: 'passport:' + passportId,
      agentId: agentId,
      displayName: card && card.name ? card.name : ('Passport ' + passportId),
      capabilities: card && Array.isArray(card.capabilities) ? card.capabilities.slice() : [],
      cardUri: cardUri || null,
      card: card,
      directEndpoint: normalizeCardEndpoint(card, 'rest'),
      websocketEndpoint: normalizeCardEndpoint(card, 'websocket'),
      passportId: passportId,
      owner: owner,
      identityRegistryAddress: identityRegistryAddress,
      sources: ['identity'],
    };
  } catch (e2) {
    return null;
  }
}

async function resolveIdentityRegistryAddress() {
  var candidates = [IDENTITY_REGISTRY_CANONICAL, IDENTITY_REGISTRY_ALIAS];
  for (var i = 0; i < candidates.length; i++) {
    var code = await rpc('eth_getCode', [candidates[i], 'latest']);
    if (code.error) continue;
    if (!isEmptyCode(code.result)) {
      return candidates[i];
    }
  }
  return null;
}

async function discoverRelayAgents() {
  var relay = await relayApi('/agents', true);
  if (!relay) return [];
  var items = Array.isArray(relay.data) ? relay.data : [];
  var agents = [];
  for (var i = 0; i < items.length; i++) {
    var raw = items[i] || {};
    var card = await resolveAgentCard(raw.card_uri || '');
    var agentId = raw.agent_id || raw.name || (card && card.name) || '';
    if (!agentId) continue;
    agents.push({
      key: 'relay:' + agentId.toLowerCase(),
      agentId: agentId,
      displayName: raw.name || (card && card.name) || agentId,
      capabilities: []
        .concat(Array.isArray(raw.capabilities) ? raw.capabilities : [])
        .concat(card && Array.isArray(card.capabilities) ? card.capabilities : []),
      cardUri: raw.card_uri || null,
      relayCardUri: raw.card_uri || null,
      card: card,
      directEndpoint: normalizeCardEndpoint(card, 'rest') || raw.rest_endpoint || null,
      websocketEndpoint: normalizeCardEndpoint(card, 'websocket'),
      relayAgentId: agentId,
      relayAvailable: true,
      relayConnected: true,
      relayBacked: !!raw.relay_backed,
      relayRestEndpoint: raw.rest_endpoint || null,
      connectedAtMs: raw.connected_at_ms || null,
      sources: ['relay'],
    });
  }
  return agents;
}

async function resolveAgentCard(cardUri) {
  if (!cardUri) return null;
  if (cardCache.has(cardUri)) return cardCache.get(cardUri);

  var card = null;
  try {
    if (cardUri.indexOf('data:application/json;base64,') === 0) {
      card = decodeCardDataUri(cardUri);
    } else {
      var resolvedUrl = cardUri;
      if (!/^https?:\/\//i.test(cardUri)) {
        resolvedUrl = joinUrl(state.remoteBase, cardUri);
      }
      var response = await requestJson(resolvedUrl, {
        label: 'card',
        message: 'GET card ' + resolvedUrl,
        softFail: true,
      });
      card = response ? response.data : null;
    }
  } catch (e) {
    card = null;
  }

  cardCache.set(cardUri, card);
  return card;
}

/* ---------- Messaging ---------- */
export async function sendAgentMessage(agent, prompt, requestedPath) {
  var route = selectTransportPath(agent, requestedPath, state.remoteBase);
  var body;

  if (route.mode === 'relay') {
    var relayResult = await relayPost('/messages', {
      agent_id: route.relayAgentId,
      message: { prompt: prompt },
      timeout_ms: 15000,
    });
    body = relayResult.data;
    return {
      transport: 'relay',
      route: route,
      body: body,
      response: body ? body.response : null,
    };
  }

  try {
    var directResult = await requestJson(joinUrl(route.endpoint, '/message'), {
      method: 'POST',
      label: 'agent',
      message: 'POST direct ' + agent.agentId,
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ prompt: prompt }),
    });
    body = directResult.data;
    return {
      transport: 'direct',
      route: route,
      body: body,
      response: body,
    };
  } catch (directError) {
    if (requestedPath === 'auto' && agent.relayAvailable) {
      logReq('warn', 'direct path failed for ' + agent.agentId + '; retrying via relay');
      return sendAgentMessage(agent, prompt, 'relay');
    }
    throw directError;
  }
}

/* ---------- Render callbacks (wired by main.js to avoid circular deps) ---------- */
var _renderLog = null;
var _renderAgent = null;
export function onRenderLog(fn) { _renderLog = fn; }
export function onRenderAgent(fn) { _renderAgent = fn; }

/* ---------- Request log ---------- */
export function logReq(lv, msg) {
  state.requestLog.push({ ts: Date.now(), lv: lv, msg: msg });
  if (state.requestLog.length > 200) state.requestLog.shift();
  if (_renderLog) _renderLog();
}

/* ---------- Agent log ---------- */
export function logAgent(type, author, msg) {
  state.agentLog.push({ ts: Date.now(), type: type, author: author, msg: msg });
  if (state.agentLog.length > 120) state.agentLog.shift();
  state.seenAuthors.add(author);
  if (_renderAgent) _renderAgent();
}

/* ---------- Toasts ---------- */
export function toast(kind, msg) {
  var el = document.createElement('div');
  el.className = 'toast ' + kind;
  el.textContent = msg;
  document.getElementById('toasts').appendChild(el);
  setTimeout(function() { el.remove(); }, 3400);
}

/* ---------- Format helpers ---------- */
export var fmtTs = function(ms) { return new Date(ms).toLocaleTimeString('en-US', {hour12:false}); };
export var shortHash = function(h) {
  if (!h) return '';
  var s = h.startsWith('insight:') ? h.slice(8) : h;
  return s.length > 14 ? s.slice(0,8) + '…' + s.slice(-4) : s;
};

export var parseHexU64 = function(s) { if (!s) return 0; var h = s.startsWith('0x') ? s.slice(2) : s; return parseInt(h, 16); };
export var parseHexBig = function(s) { if (!s) return 0n; var h = s.startsWith('0x') ? s.slice(2) : s; return BigInt('0x'+h); };
export var weiToGwei = function(wei) { return Number(wei) / 1e9; };
