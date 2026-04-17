/* ================================================================
   AGENT REGISTRY CORE — remote-base normalization, ERC-8004 reads,
   source merge rules, and transport selection shared by UI + smoke
   ================================================================ */

export var DEFAULT_REMOTE_BASE = 'http://127.0.0.1:8545';
export var IDENTITY_REGISTRY_CANONICAL = '0x8004A818BFB912233c491871b3d84c89A494Bd9E';
export var IDENTITY_REGISTRY_ALIAS = '0x000000000000000000000000000000000000A100';
export var PASSPORT_MINTED_TOPIC =
  '0xc32aa25f2a8ebb22010dfee1b3af98d0628f080e9398c9897a46fed69f416fa9';
export var OWNER_OF_SELECTOR = '0x6352211e';
export var TOKEN_URI_SELECTOR = '0xc87b56dd';

var PRIVATE_HOST_PATTERNS = [
  /^localhost$/i,
  /^127\./,
  /^0\.0\.0\.0$/,
  /^\[::1\]$/i,
  /^10\./,
  /^192\.168\./,
  /^172\.(1[6-9]|2\d|3[0-1])\./,
];

export function normalizeRemoteBase(input) {
  var raw = (input || '').trim();
  if (!raw) {
    raw = defaultBrowserBase();
  }
  if (!hasScheme(raw) && raw[0] !== '/' && raw.indexOf('.') !== -1) {
    raw = 'http://' + raw;
  }
  var url = new URL(raw, defaultBrowserBase());
  var pathname = url.pathname.replace(/\/+$/, '');
  if (
    pathname === '/dashboard' ||
    pathname === '/api' ||
    pathname === '/relay'
  ) {
    pathname = '';
  } else if (pathname.endsWith('/dashboard/index.html')) {
    pathname = pathname.slice(0, -'/dashboard/index.html'.length);
  } else if (pathname.endsWith('/dashboard')) {
    pathname = pathname.slice(0, -'/dashboard'.length);
  } else if (pathname.endsWith('/index.html')) {
    pathname = pathname.slice(0, -'/index.html'.length);
  } else if (pathname.endsWith('/api')) {
    pathname = pathname.slice(0, -'/api'.length);
  } else if (pathname.endsWith('/relay')) {
    pathname = pathname.slice(0, -'/relay'.length);
  }
  url.pathname = pathname || '/';
  url.search = '';
  url.hash = '';
  return url.toString().replace(/\/$/, '');
}

export function deriveRemoteBases(remoteBase) {
  var normalized = normalizeRemoteBase(remoteBase);
  var wsBase = normalized
    .replace(/^http:\/\//, 'ws://')
    .replace(/^https:\/\//, 'wss://');
  return {
    remoteBase: normalized,
    rpcUrl: normalized,
    apiBase: joinUrl(normalized, '/api'),
    relayBase: joinUrl(normalized, '/relay'),
    wsBase: wsBase,
    wsUrl: joinUrl(wsBase, '/api/ws'),
  };
}

export function joinUrl(base, path) {
  var root = (base || '').replace(/\/+$/, '');
  var suffix = path || '';
  if (!suffix) return root;
  if (suffix[0] !== '/') suffix = '/' + suffix;
  return root + suffix;
}

export function isEmptyCode(code) {
  return !code || code === '0x' || code === '0x0' || code === '0x00';
}

export function encodeUint256(value) {
  var big = typeof value === 'bigint' ? value : BigInt(value);
  var hex = big.toString(16);
  while (hex.length < 64) hex = '0' + hex;
  return hex;
}

export function buildCallData(selector, value) {
  return selector + encodeUint256(value);
}

export function parseTopicUint(topic) {
  if (!topic) return 0;
  return Number(BigInt(topic));
}

export function decodeAddressResult(hex) {
  var body = strip0x(hex);
  if (body.length < 64) return null;
  return '0x' + body.slice(body.length - 40);
}

export function decodeStringResult(hex) {
  var body = strip0x(hex);
  if (!body) return '';
  if (body.length < 128) {
    return hexToUtf8(body.replace(/0+$/, ''));
  }
  var offset = Number(BigInt('0x' + body.slice(0, 64))) * 2;
  if (offset + 64 > body.length) return '';
  var length = Number(BigInt('0x' + body.slice(offset, offset + 64))) * 2;
  var start = offset + 64;
  var end = start + length;
  if (end > body.length) return '';
  return hexToUtf8(body.slice(start, end));
}

export function decodeCardDataUri(uri) {
  var prefix = 'data:application/json;base64,';
  if (!uri || uri.indexOf(prefix) !== 0) return null;
  var payload = uri.slice(prefix.length);
  var json = decodeBase64(payload);
  return JSON.parse(json);
}

export function mergeAgentSources(identityAgents, relayAgents) {
  var merged = [];
  var byAgentId = new Map();
  var byCardUri = new Map();
  var byPassport = new Map();

  function indexAgent(agent) {
    if (agent.agentId) byAgentId.set(agent.agentId.toLowerCase(), agent);
    if (agent.cardUri) byCardUri.set(agent.cardUri.toLowerCase(), agent);
    if (agent.passportId != null) byPassport.set(String(agent.passportId), agent);
  }

  function findExisting(agent) {
    if (agent.agentId && byAgentId.has(agent.agentId.toLowerCase())) {
      return byAgentId.get(agent.agentId.toLowerCase());
    }
    if (agent.cardUri && byCardUri.has(agent.cardUri.toLowerCase())) {
      return byCardUri.get(agent.cardUri.toLowerCase());
    }
    if (agent.passportId != null && byPassport.has(String(agent.passportId))) {
      return byPassport.get(String(agent.passportId));
    }
    return null;
  }

  function ensureAgent(partial) {
    var existing = findExisting(partial);
    if (existing) {
      applyPartial(existing, partial);
      indexAgent(existing);
      return existing;
    }
    var created = {
      key: partial.key || fallbackAgentKey(partial),
      agentId: partial.agentId || '',
      displayName: partial.displayName || partial.agentId || 'unknown-agent',
      capabilities: uniqueStrings(partial.capabilities || []),
      cardUri: partial.cardUri || null,
      card: partial.card || null,
      directEndpoint: partial.directEndpoint || null,
      websocketEndpoint: partial.websocketEndpoint || null,
      passportId: partial.passportId != null ? partial.passportId : null,
      owner: partial.owner || null,
      identityRegistryAddress: partial.identityRegistryAddress || null,
      relayAgentId: partial.relayAgentId || partial.agentId || null,
      relayAvailable: !!partial.relayAvailable,
      relayConnected: !!partial.relayConnected,
      relayBacked: !!partial.relayBacked,
      relayCardUri: partial.relayCardUri || null,
      relayRestEndpoint: partial.relayRestEndpoint || null,
      connectedAtMs: partial.connectedAtMs || null,
      sources: uniqueStrings(partial.sources || []),
    };
    merged.push(created);
    indexAgent(created);
    return created;
  }

  function applyPartial(target, partial) {
    if (!target.agentId && partial.agentId) target.agentId = partial.agentId;
    if ((!target.displayName || target.displayName === 'unknown-agent') && partial.displayName) {
      target.displayName = partial.displayName;
    }
    target.capabilities = uniqueStrings(target.capabilities.concat(partial.capabilities || []));
    if (!target.cardUri && partial.cardUri) target.cardUri = partial.cardUri;
    if (!target.card && partial.card) target.card = partial.card;
    if (!target.directEndpoint && partial.directEndpoint) target.directEndpoint = partial.directEndpoint;
    if (!target.websocketEndpoint && partial.websocketEndpoint) {
      target.websocketEndpoint = partial.websocketEndpoint;
    }
    if (target.passportId == null && partial.passportId != null) target.passportId = partial.passportId;
    if (!target.owner && partial.owner) target.owner = partial.owner;
    if (!target.identityRegistryAddress && partial.identityRegistryAddress) {
      target.identityRegistryAddress = partial.identityRegistryAddress;
    }
    if (!target.relayAgentId && partial.relayAgentId) target.relayAgentId = partial.relayAgentId;
    target.relayAvailable = target.relayAvailable || !!partial.relayAvailable;
    target.relayConnected = target.relayConnected || !!partial.relayConnected;
    target.relayBacked = target.relayBacked || !!partial.relayBacked;
    if (!target.relayCardUri && partial.relayCardUri) target.relayCardUri = partial.relayCardUri;
    if (!target.relayRestEndpoint && partial.relayRestEndpoint) {
      target.relayRestEndpoint = partial.relayRestEndpoint;
    }
    if (!target.connectedAtMs && partial.connectedAtMs) target.connectedAtMs = partial.connectedAtMs;
    target.sources = uniqueStrings(target.sources.concat(partial.sources || []));
    if (target.card && target.card.name && target.displayName !== target.card.name) {
      target.displayName = target.card.name;
      if (!target.agentId) target.agentId = target.card.name;
    }
  }

  for (var i = 0; i < identityAgents.length; i++) {
    ensureAgent(identityAgents[i]);
  }
  for (var j = 0; j < relayAgents.length; j++) {
    ensureAgent(relayAgents[j]);
  }

  merged.sort(function(left, right) {
    return (left.displayName || left.agentId || '').localeCompare(
      right.displayName || right.agentId || '',
      'en',
      { sensitivity: 'base' }
    );
  });
  return merged;
}

export function transportOptionsForAgent(agent, remoteBase) {
  var options = [{ value: 'auto', label: 'auto' }];
  if (agent && agent.directEndpoint) {
    options.push({ value: 'direct', label: 'direct' });
  }
  if (agent && agent.relayAvailable) {
    options.push({ value: 'relay', label: 'relay' });
  }
  return options;
}

export function selectTransportPath(agent, requested, remoteBase) {
  if (!agent) {
    throw new Error('select an agent first');
  }
  var preferred = requested || 'auto';
  if (preferred === 'direct') {
    if (!agent.directEndpoint) {
      throw new Error('agent has no direct endpoint');
    }
    return {
      mode: 'direct',
      endpoint: agent.directEndpoint,
      browserReachable: isBrowserReachableDirect(agent.directEndpoint, remoteBase),
    };
  }
  if (preferred === 'relay') {
    if (!agent.relayAvailable || !agent.relayAgentId) {
      throw new Error('agent is not relay reachable');
    }
    return {
      mode: 'relay',
      relayAgentId: agent.relayAgentId,
    };
  }
  if (agent.directEndpoint && isBrowserReachableDirect(agent.directEndpoint, remoteBase)) {
    return {
      mode: 'direct',
      endpoint: agent.directEndpoint,
      browserReachable: true,
    };
  }
  if (agent.relayAvailable && agent.relayAgentId) {
    return {
      mode: 'relay',
      relayAgentId: agent.relayAgentId,
    };
  }
  if (agent.directEndpoint) {
    return {
      mode: 'direct',
      endpoint: agent.directEndpoint,
      browserReachable: false,
    };
  }
  throw new Error('agent has no usable transport');
}

export function describeAgentSources(agent) {
  if (!agent) return 'none';
  if (agent.passportId != null && agent.relayAvailable) return 'identity + relay';
  if (agent.passportId != null) return 'identity';
  if (agent.relayAvailable) return 'relay';
  return 'unresolved';
}

export function describeTransportSummary(agent, remoteBase) {
  if (!agent) return 'none';
  var lanes = [];
  if (agent.directEndpoint) {
    lanes.push(
      isBrowserReachableDirect(agent.directEndpoint, remoteBase)
        ? 'direct-ready'
        : 'direct-manual'
    );
  }
  if (agent.relayAvailable) lanes.push('relay-live');
  return lanes.length ? lanes.join(' + ') : 'no-route';
}

export function isBrowserReachableDirect(endpoint, remoteBase) {
  try {
    var direct = new URL(endpoint, remoteBase);
    var base = new URL(normalizeRemoteBase(remoteBase));
    if (direct.origin === base.origin) return true;
    return !isPrivateHostname(direct.hostname);
  } catch (e) {
    return false;
  }
}

export function isPrivateHostname(hostname) {
  var normalized = (hostname || '').toLowerCase();
  for (var i = 0; i < PRIVATE_HOST_PATTERNS.length; i++) {
    if (PRIVATE_HOST_PATTERNS[i].test(normalized)) return true;
  }
  return false;
}

export function normalizeCardEndpoint(card, field) {
  if (!card || !card.endpoints || typeof card.endpoints !== 'object') return null;
  var value = card.endpoints[field];
  return typeof value === 'string' && value.trim() ? value.trim() : null;
}

function uniqueStrings(values) {
  var out = [];
  var seen = new Set();
  for (var i = 0; i < values.length; i++) {
    var value = values[i];
    if (typeof value !== 'string') continue;
    var normalized = value.trim();
    if (!normalized || seen.has(normalized)) continue;
    seen.add(normalized);
    out.push(normalized);
  }
  return out;
}

function fallbackAgentKey(agent) {
  if (agent.agentId) return 'agent:' + agent.agentId.toLowerCase();
  if (agent.cardUri) return 'card:' + agent.cardUri.toLowerCase();
  if (agent.passportId != null) return 'passport:' + agent.passportId;
  return 'agent:unknown';
}

function decodeBase64(payload) {
  if (typeof atob === 'function') {
    return decodeURIComponent(escape(atob(payload)));
  }
  if (typeof Buffer !== 'undefined') {
    return Buffer.from(payload, 'base64').toString('utf8');
  }
  throw new Error('base64 decoding is unavailable in this runtime');
}

function hexToUtf8(hex) {
  if (!hex) return '';
  var bytes = [];
  for (var i = 0; i < hex.length; i += 2) {
    bytes.push(parseInt(hex.slice(i, i + 2), 16));
  }
  if (typeof TextDecoder !== 'undefined') {
    return new TextDecoder().decode(new Uint8Array(bytes));
  }
  if (typeof Buffer !== 'undefined') {
    return Buffer.from(bytes).toString('utf8');
  }
  return '';
}

function strip0x(hex) {
  return typeof hex === 'string' && hex.indexOf('0x') === 0 ? hex.slice(2) : (hex || '');
}

function hasScheme(value) {
  return /^[a-z][a-z0-9+.-]*:\/\//i.test(value);
}

function defaultBrowserBase() {
  if (typeof window !== 'undefined' && window.location && window.location.origin) {
    return window.location.origin;
  }
  return DEFAULT_REMOTE_BASE;
}
