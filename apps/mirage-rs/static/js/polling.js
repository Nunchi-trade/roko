/* ================================================================
   POLLING — all poll* functions (block, chain, entries, edges,
             kinds, pheroSummary, heatmap, topology)
   ================================================================ */

import {
  applyAgentDiscovery,
  getSelectedMergedAgent,
  listTransportOptionsForAgent,
  setSelectedMergedAgent,
  state,
} from './state.js';
import {
  api,
  discoverAgentRegistry,
  logAgent,
  parseHexBig,
  parseHexU64,
  rpc,
  weiToGwei,
  fmtTs,
} from './api.js';
import { describeAgentSources, describeTransportSummary, isBrowserReachableDirect } from './agent_registry.js';
import { pushSeries, renderBlocks, updateHero } from './charts.js';
import { depositPheromoneParticle, P_HALFLIFE } from './pheromones.js';
import { addInsightNode, graphNodes, graphEdges } from './graph.js';

/* ---------- Block polling (still JSON-RPC: eth_*) ---------- */
export async function pollBlock() {
  try {
    var result = await rpc('eth_getBlockByNumber', ['latest', false]);
    var blk = result.result;
    if (!blk) return;
    var num = parseHexU64(blk.number);
    var b = {
      number: num,
      hash: blk.hash,
      timestamp: parseHexU64(blk.timestamp),
      gasUsed: parseHexU64(blk.gasUsed),
      gasLimit: parseHexU64(blk.gasLimit),
      baseFeeGwei: blk.baseFeePerGas ? weiToGwei(parseHexBig(blk.baseFeePerGas)) : 0,
      txCount: Array.isArray(blk.transactions) ? blk.transactions.length : 0,
      fresh: true,
    };
    b.saturation = b.gasLimit > 0 ? (b.gasUsed / b.gasLimit) * 100 : 0;
    if (state.blocks.length && state.blocks[state.blocks.length-1].number >= num) return;
    state.blocks.forEach(function(x) { x.fresh = false; });
    state.blocks.push(b);
    if (state.blocks.length > 60) state.blocks.shift();
    renderBlocks();
    updateHero();
  } catch (e) { /* ignore */ }
}

/* ---------- Chain polling — REST API ---------- */
export async function pollChain() {
  try {
    // GET /api/stats — replaces chain_stats
    var statsRes = await api('/stats');
    var stats = statsRes.data;
    if (stats) {
      var prev = state.insightsTotalPrev || 0;
      var insightsTotal = stats.insights ? stats.insights.total : 0;
      var pheroTotal = stats.pheromones ? stats.pheromones.total : 0;
      state.insightsTotalPrev = insightsTotal;
      state.chainInsightsTotal = insightsTotal;
      state.chainPheromonesTotal = pheroTotal;
      document.getElementById('h-insights').textContent = insightsTotal;
      document.getElementById('h-insights-delta').textContent = '+' + (insightsTotal - prev) + '/poll';
      document.getElementById('h-phero').textContent = pheroTotal;
      if (stats.pheromones) {
        document.getElementById('phero-meta').textContent =
          pheroTotal + ' live · threat:' + (stats.pheromones.threat || 0) +
          ' opp:' + (stats.pheromones.opportunity || 0) +
          ' wisdom:' + (stats.pheromones.wisdom || 0);
      }
    }
    // GET /api/knowledge/search — replaces chain_searchInsights for polling
    var searchQueries = [
      'gas saturation base fee congestion spike',
      'whale transfer large ETH move',
      'DEX swap uniswap sushiswap curve liquidity',
      'lending aave compound borrow supply',
      'stablecoin USDC USDT DAI velocity',
      'MEV sandwich priority tip flashbot',
      'bridge cross-chain arbitrum optimism L2',
      'NFT marketplace opensea ERC721',
      'contract deploy upgrade proxy',
      'threat opportunity wisdom convergence pattern',
    ];
    var sq = searchQueries[state.rpc.total % searchQueries.length];
    var searchRes = await api('/knowledge/search?q=' + encodeURIComponent(sq) + '&k=60');
    var searchData = searchRes.data;
    if (searchData && searchData.results) {
      var totalConf = 0, totalChall = 0;
      for (var i = 0; i < searchData.results.length; i++) {
        totalConf += searchData.results[i].confirmations || 0;
        totalChall += searchData.results[i].challenges || 0;
      }
      state.observedConfirms = Math.max(state.observedConfirms || 0, totalConf);
      state.observedChallenges = Math.max(state.observedChallenges || 0, totalChall);
      state.confirmsCount = Math.max(state.confirmsCount, state.observedConfirms);
      state.challengesCount = Math.max(state.challengesCount, state.observedChallenges);

      for (var si = 0; si < searchData.results.length; si++) {
        var hit = searchData.results[si];
        var id = (hit.id || '').replace(/^insight:/, '');
        if (!id) continue;
        var existing = state.insights.get(id);
        var row = {
          id: id, kind: hit.kind || 'insight', content: hit.content || '',
          author: existing ? existing.author : null, conf: hit.confirmations || 0,
          chall: hit.challenges || 0, weight: hit.weight || 1.0, createdAt: Date.now(),
          similarity: hit.similarity || 0, score: hit.score || 0, state: hit.state,
        };
        if (!existing) {
          state.insights.set(id, row);
          addInsightNode(id, row.kind, row.content);
          // Fetch author in background via JSON-RPC (mutation-adjacent)
          (function(capturedRow, capturedId) {
            rpc('chain_getInsight', {id: 'insight:' + capturedId}).then(function(res) {
              var full = res.result;
              if (full && full.author) {
                capturedRow.author = full.author;
                state.seenAuthors.add(full.author);
                var n = graphNodes.find(function(x) { return x.id === capturedId; });
                if (n) n.author = full.author;
                logAgent('observe', full.author, capturedRow.content.slice(0, 80));
              }
            }).catch(function() {});
          })(row, id);
        } else {
          var confDelta = row.conf - existing.conf;
          var challDelta = row.chall - existing.chall;
          if (confDelta > 0) {
            state.confirmsCount += confDelta;
            if (existing.author) logAgent('confirm', 'chain', existing.author + "'s insight confirmed (+" + confDelta + ')');
          }
          if (challDelta > 0) {
            state.challengesCount += challDelta;
            if (existing.author) logAgent('challenge', 'chain', existing.author + "'s insight challenged (+" + challDelta + ')');
          }
          existing.conf = row.conf; existing.chall = row.chall; existing.weight = row.weight;
          var gn = graphNodes.find(function(x) { return x.id === id; });
          if (gn) { gn.conf = row.conf; gn.chall = row.chall; gn.pulse = Math.max(gn.pulse, 0.6); }
        }
      }
      // Search metric
      document.getElementById('m-search').textContent = Math.round(searchRes.ms);
      document.getElementById('m-search-count').textContent = searchData.results.length + ' hits';
      pushSeries('search', searchRes.ms);
    }

    // GET /api/pheromones — fetch all kinds, re-inject particles for alive chain pheromones
    // Handle PaginatedResponse envelope: {items:[...], total, offset, limit, has_more}
    var pheroRes = await api('/pheromones?limit=200&sort=deposited_at&order=desc');
    var pheroData = pheroRes.data;
    var pheromoneList = (pheroData && pheroData.items) || (pheroData && pheroData.pheromones) || [];
    if (pheromoneList.length) {
      // Build set of chain IDs that currently have a LIVE particle (pulse > 0.02)
      var liveParticles = new Set();
      for (var lp = 0; lp < state.pheromones.length; lp++) {
        if (state.pheromones[lp].chainId && state.pheromones[lp].pulse > 0.02) {
          liveParticles.add(state.pheromones[lp].chainId);
        }
      }
      for (var pi = 0; pi < pheromoneList.length; pi++) {
        var h = pheromoneList[pi];
        if (liveParticles.has(h.id)) {
          // Update decay projections on existing live particles
          var existingP = state.pheromones.find(function(p) { return p.chainId === h.id; });
          if (existingP && h.decay_projection) {
            existingP.decayProjection = h.decay_projection;
          }
          continue;
        }
        // Particle either doesn't exist or died — re-inject with real chain intensity
        depositPheromoneParticle(h.kind, '#' + h.id, h.intensity || 0.7, h.id);
        var newP = state.pheromones[state.pheromones.length - 1];
        if (newP) {
          newP.decayProjection = h.decay_projection || null;
          // Use longer visual half-life so particles stay visible between polls
          newP.halfLife = Math.max(P_HALFLIFE[h.kind] || 60, 45);
        }
      }
    }
  } catch (e) { /* ignore */ }
}

/* ---------- Poll knowledge entries via REST ---------- */
export async function pollEntries() {
  try {
    var res = await api('/knowledge/entries?limit=400&sort=created_at&order=desc');
    var data = res.data;
    // Handle PaginatedResponse envelope: {items:[...], total, offset, limit, has_more}
    var entries = (data && data.items) || (data && data.entries) || [];
    for (var i = 0; i < entries.length; i++) {
      var entry = entries[i];
      var id = (entry.id || '').replace(/^insight:/, '');
      if (!id) continue;
      var existing = state.insights.get(id);
      if (!existing) {
        var row = {
          id: id, kind: entry.kind || 'insight', content: entry.content || '',
          author: entry.author || null, conf: entry.confirmations || 0,
          chall: entry.challenges || 0, weight: entry.weight || 1.0, createdAt: Date.now(),
          similarity: 0, score: 0, state: entry.state,
        };
        state.insights.set(id, row);
        addInsightNode(id, row.kind, row.content, { author: entry.author });
        if (entry.author) state.seenAuthors.add(entry.author);
      } else {
        existing.conf = entry.confirmations || existing.conf;
        existing.chall = entry.challenges || existing.chall;
        existing.weight = entry.weight || existing.weight;
        if (entry.author && !existing.author) {
          existing.author = entry.author;
          state.seenAuthors.add(entry.author);
        }
        var gn = graphNodes.find(function(x) { return x.id === id; });
        if (gn) {
          gn.conf = existing.conf; gn.chall = existing.chall;
          if (entry.author) gn.author = entry.author;
        }
      }
    }
  } catch (e) { /* ignore */ }
}

/* ---------- Poll knowledge edges via REST ---------- */
export async function pollEdges() {
  try {
    var res = await api('/knowledge/edges?similarity_threshold=0.4&max_hdc_edges_per_node=5&include_enabled_by=true&include_hdc=true');
    var data = res.data;
    // Handle PaginatedResponse envelope: {items:[...], total, offset, limit, has_more}
    var edges = (data && data.items) || (data && data.edges) || [];
    // Remove old REST-sourced edges, keep user-interaction edges (hdc from click)
    for (var i = graphEdges.length - 1; i >= 0; i--) {
      if (graphEdges[i].source === 'rest') graphEdges.splice(i, 1);
    }
    for (var ei = 0; ei < edges.length; ei++) {
      var e = edges[ei];
      var fromId = (e.from || '').replace(/^insight:/, '');
      var toId = (e.to || '').replace(/^insight:/, '');
      if (!fromId || !toId || fromId === toId) continue;
      var fromNode = graphNodes.find(function(n) { return n.id === fromId; });
      var toNode = graphNodes.find(function(n) { return n.id === toId; });
      if (!fromNode || !toNode) continue;
      var already = graphEdges.find(function(x) {
        return (x.from === fromId && x.to === toId) || (x.from === toId && x.to === fromId);
      });
      if (already) continue;
      var kind = e.type === 'enabled_by' ? 'enabled_by' : 'hdc';
      graphEdges.push({ from: fromId, to: toId, kind: kind, similarity: e.similarity || 0, source: 'rest' });
    }
    while (graphEdges.length > 500) graphEdges.shift();
  } catch (e) { /* ignore */ }
}

/* ---------- Poll pheromone summary via REST ---------- */
export async function pollPheroSummary() {
  try {
    var res = await api('/pheromones/summary');
    var data = res.data;
    if (data) {
      var kinds = ['threat', 'opportunity', 'wisdom'];
      var prefixes = ['ps-threat', 'ps-opp', 'ps-wisdom'];
      for (var i = 0; i < kinds.length; i++) {
        var kd = (data.by_kind || {})[kinds[i]] || {};
        var pfx = prefixes[i];
        document.getElementById(pfx + '-count').textContent = kd.count || 0;
        document.getElementById(pfx + '-total').textContent = (kd.total_intensity || 0).toFixed(2);
        document.getElementById(pfx + '-avg').textContent = (kd.avg_intensity || 0).toFixed(2);
      }
      document.getElementById('phero-summary-meta').textContent =
        'updated ' + fmtTs(Date.now()) + ' · ' + Math.round(res.ms) + 'ms';
    }
  } catch (e) { /* ignore */ }
}

/* ---------- Poll pheromone heatmap via REST ---------- */
export async function pollHeatmap() {
  try {
    var since = Math.floor(Date.now() / 1000) - 3600;
    var res = await api('/pheromones/heatmap?bucket_seconds=300&since=' + since);
    var data = res.data;
    if (data && data.buckets) {
      state.heatmapBuckets = data.buckets;
      document.getElementById('heatmap-meta').textContent =
        data.buckets.length + ' buckets · ' + Math.round(res.ms) + 'ms';
      // Update hud totals
      var tT = 0, tO = 0, tW = 0;
      for (var i = 0; i < data.buckets.length; i++) {
        tT += data.buckets[i].threat || 0;
        tO += data.buckets[i].opportunity || 0;
        tW += data.buckets[i].wisdom || 0;
      }
      document.getElementById('hm-threat').textContent = tT;
      document.getElementById('hm-opp').textContent = tO;
      document.getElementById('hm-wisdom').textContent = tW;
    }
  } catch (e) { /* ignore */ }
}

/* ---------- Poll agent topology via REST ---------- */
export async function pollTopology() {
  try {
    var res = await api('/agents/topology');
    var data = res.data;
    if (!data) return;
    // Preserve existing positions
    var oldPos = {};
    for (var i = 0; i < state.topoNodes.length; i++) {
      oldPos[state.topoNodes[i].id] = { x: state.topoNodes[i].x, y: state.topoNodes[i].y, vx: state.topoNodes[i].vx, vy: state.topoNodes[i].vy };
    }
    var nodes = data.nodes || [];
    var edges = data.edges || [];
    var topoCanvas = document.getElementById('topo-canvas');
    var w = topoCanvas.clientWidth || 600;
    var h = topoCanvas.clientHeight || 400;
    state.topoNodes = [];
    for (var ni = 0; ni < nodes.length; ni++) {
      var n = nodes[ni];
      var old = oldPos[n.id];
      state.topoNodes.push({
        id: n.id,
        role: n.role || 'agent',
        insightsPosted: n.insights_posted || 0,
        confirmationsGiven: n.confirmations_given || 0,
        challengesGiven: n.challenges_given || 0,
        totalWeight: n.total_weight || 0,
        x: old ? old.x : w/2 + (Math.random()-0.5)*w*0.85,
        y: old ? old.y : h/2 + (Math.random()-0.5)*h*0.85,
        vx: old ? old.vx : 0,
        vy: old ? old.vy : 0,
      });
    }
    state.topoEdges = [];
    for (var ei = 0; ei < edges.length; ei++) {
      var e = edges[ei];
      state.topoEdges.push({
        from: e.from,
        to: e.to,
        weight: e.weight || 1,
        type: e.type || 'confirmed',
      });
    }
    document.getElementById('t-agents').textContent = state.topoNodes.length;
    document.getElementById('t-links').textContent = state.topoEdges.length;
    document.getElementById('topo-meta').textContent =
      state.topoNodes.length + ' agents · ' + state.topoEdges.length + ' links · ' + Math.round(res.ms) + 'ms';
    // Leaderboard updated separately by pollLeaderboard()
  } catch (e) { /* ignore */ }
}

/* ---------- Poll knowledge kinds via REST ---------- */
export async function pollKinds() {
  try {
    var res = await api('/knowledge/kinds');
    var data = res.data;
    if (data) {
      state.kindsData = data;
      var tbody = document.getElementById('kinds-tbody');
      tbody.innerHTML = '';
      var kinds = data.kinds || [];
      var registries = data.registries || {};
      var totalCount = 0;
      for (var ki = 0; ki < kinds.length; ki++) {
        var k = kinds[ki];
        var r = registries[k.name] || {};
        var count = k.count || 0;
        totalCount += count;
        var tr = document.createElement('tr');
        var hlLabel = r.half_life ? (r.half_life / 3600).toFixed(1) + 'h' : '—';
        var reward = r.reward_per_confirmation ? (r.reward_per_confirmation / 1e18).toFixed(6) : '—';
        tr.innerHTML =
          '<td class="kind-name">' + (r.name || '—') + '</td>' +
          '<td>' + k.type + '</td>' +
          '<td>' + hlLabel + '</td>' +
          '<td>' + reward + '</td>' +
          '<td style="color:var(--text);font-weight:600">' + count + '</td>';
        tbody.appendChild(tr);
      }
      document.getElementById('kinds-total-label').innerHTML = totalCount + '<span class="unit"> entries</span>';
    }
  } catch (e) { /* ignore */ }
}

/* ---------- Agent registry via ERC-8004 + relay ---------- */
var agentRegHash = '';

function esc(value) {
  return String(value == null ? '' : value)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function shortUri(uri) {
  if (!uri) return '—';
  return uri.length > 48 ? uri.slice(0, 32) + '…' + uri.slice(-12) : uri;
}

function renderRegistryRows(agents) {
  var tbody = document.getElementById('agent-reg-tbody');
  if (!tbody) return;
  if (!agents.length) {
    tbody.innerHTML = '<tr><td colspan="8" style="color:var(--text-faint)">no agents discovered from identity registry or relay</td></tr>';
    return;
  }

  var html = '';
  for (var i = 0; i < agents.length; i++) {
    var agent = agents[i];
    var pathSummary = describeTransportSummary(agent, state.remoteBase);
    var sourceSummary = describeAgentSources(agent);
    var directSummary = agent.directEndpoint
      ? (isBrowserReachableDirect(agent.directEndpoint, state.remoteBase) ? 'ready' : 'manual')
      : '—';
    html += '<tr data-agent-key="' + esc(agent.key) + '">' +
      '<td style="font-weight:600;color:var(--accent-bright)">' + esc(agent.displayName || agent.agentId) + '</td>' +
      '<td>' + (agent.passportId != null ? ('#' + esc(agent.passportId)) : 'wallet-free') + '</td>' +
      '<td>' + esc(sourceSummary) + '</td>' +
      '<td>' + esc(pathSummary) + '</td>' +
      '<td>' + esc((agent.capabilities || []).join(', ') || '—') + '</td>' +
      '<td title="' + esc(agent.cardUri || '') + '">' + esc(shortUri(agent.cardUri)) + '</td>' +
      '<td>' + esc(directSummary) + '</td>' +
      '<td><button class="btn ghost sm" data-agent-key="' + esc(agent.key) + '">MESSAGE</button></td>' +
      '</tr>';
  }
  tbody.innerHTML = html;
}

function renderComposerTargets() {
  var select = document.getElementById('msg-agent');
  if (!select) return;
  var agents = state.registry.mergedAgents;
  select.innerHTML = '';
  if (!agents.length) {
    var empty = document.createElement('option');
    empty.value = '';
    empty.textContent = 'no discovered agents';
    select.appendChild(empty);
    return;
  }
  for (var i = 0; i < agents.length; i++) {
    var option = document.createElement('option');
    option.value = agents[i].key;
    option.textContent = agents[i].displayName || agents[i].agentId;
    select.appendChild(option);
  }
  var selected = getSelectedMergedAgent() || agents[0];
  if (selected) {
    setSelectedMergedAgent(selected.key);
    select.value = selected.key;
  }
}

function renderTransportOptions() {
  var select = document.getElementById('msg-path');
  if (!select) return;
  var agent = getSelectedMergedAgent();
  var options = listTransportOptionsForAgent(agent);
  select.innerHTML = '';
  for (var i = 0; i < options.length; i++) {
    var option = document.createElement('option');
    option.value = options[i].value;
    option.textContent = options[i].label.toUpperCase();
    select.appendChild(option);
  }
  select.value = state.registry.transportPreference;
  if (select.value !== state.registry.transportPreference) {
    select.value = 'auto';
  }
}

function updateRegistryMeta(agents, discovery) {
  document.getElementById('h-agents').textContent = agents.length;
  var meta = agents.length + ' merged · '
    + discovery.identityAgents.length + ' on-chain · '
    + discovery.relayAgents.length + ' relay-live';
  if (state.registry.identityRegistryAddress) {
    meta += ' · registry ' + state.registry.identityRegistryAddress.slice(0, 10) + '…';
  }
  document.getElementById('agent-reg-meta').textContent = meta;
  var chip = document.getElementById('agents-chip');
  if (chip) {
    chip.className = agents.length > 0 ? 'chip ok' : 'chip';
    chip.innerHTML = '<span class="dot"></span>' + agents.length + ' agents';
  }
  pushSeries('agents', agents.length);
}

export async function pollAgentRegistry() {
  try {
    var previousKeys = {};
    for (var i = 0; i < state.registry.mergedAgents.length; i++) {
      previousKeys[state.registry.mergedAgents[i].key] = true;
    }

    var discovery = await discoverAgentRegistry();
    var agents = applyAgentDiscovery(discovery);
    for (var j = 0; j < agents.length; j++) {
      state.seenAuthors.add(agents[j].agentId);
      if (!previousKeys[agents[j].key]) {
        logAgent('observe', 'registry', 'discovered ' + (agents[j].displayName || agents[j].agentId));
      }
    }

    updateRegistryMeta(agents, discovery);

    var hash = agents.map(function(agent) {
      return [
        agent.key,
        agent.passportId,
        agent.cardUri,
        agent.relayAvailable,
        agent.directEndpoint,
      ].join(':');
    }).join('|');
    if (hash === agentRegHash) return;
    agentRegHash = hash;

    renderRegistryRows(agents);
    renderComposerTargets();
    renderTransportOptions();
    if (typeof window !== 'undefined' && typeof window.dispatchEvent === 'function') {
      window.dispatchEvent(new CustomEvent('registry-updated'));
    }
  } catch (e) {
    document.getElementById('agent-reg-meta').textContent = 'discovery failed: ' + e.message;
  }
}

/* ---------- Poll leaderboard from topology (diff-update) ---------- */
var leaderHash = '';
export async function pollLeaderboard() {
  try {
    var res = await api('/agents/topology');
    var data = res.data;
    if (!data || !data.nodes) return;
    var nodes = data.nodes.slice().sort(function(a, b) {
      return (b.insights_posted || 0) - (a.insights_posted || 0);
    });

    var hash = nodes.map(function(n) { return n.id + (n.insights_posted || 0); }).join('|');
    if (hash === leaderHash) return;
    leaderHash = hash;

    var tbody = document.getElementById('leaderboard-tbody');
    if (!tbody) return;
    document.getElementById('leaderboard-meta').textContent = nodes.length + ' agents';

    // Clear placeholder row (has colspan, not 5 separate tds)
    if (tbody.children.length > 0 && tbody.children[0].children.length !== 5) {
      tbody.innerHTML = '';
    }

    while (tbody.children.length > nodes.length) tbody.removeChild(tbody.lastChild);
    while (tbody.children.length < nodes.length) {
      var tr = document.createElement('tr');
      for (var c = 0; c < 5; c++) tr.appendChild(document.createElement('td'));
      tbody.appendChild(tr);
    }
    if (nodes.length === 0) {
      tbody.innerHTML = '<tr><td colspan="5" style="color:var(--text-faint)">no agents</td></tr>';
      return;
    }
    for (var i = 0; i < nodes.length; i++) {
      var n = nodes[i];
      var row = tbody.children[i];
      var cells = row.children;
      cells[0].textContent = n.id;
      cells[0].style.cssText = 'font-weight:600;color:var(--accent-bright)';
      cells[1].textContent = n.insights_posted || n.insightsPosted || 0;
      cells[2].textContent = n.confirmations_given || n.confirmationsGiven || 0;
      cells[2].style.color = 'var(--green)';
      cells[3].textContent = n.challenges_given || n.challengesGiven || 0;
      cells[3].style.color = 'var(--red)';
      cells[4].textContent = (n.total_weight || n.totalWeight || 0).toFixed(2);
    }
  } catch (e) { /* ignore */ }
}

/* ---------- Poll task lifecycle via REST ---------- */
var taskHash = '';
export async function pollTasks() {
  try {
    // Try task stats endpoint first
    var statsRes = await api('/tasks/stats');
    var stats = statsRes.data;
    if (stats) {
      var el = function(id, v) { var e = document.getElementById(id); if (e) e.textContent = v; };
      el('ts-open', stats.open || 0);
      el('ts-assigned', stats.assigned || 0);
      el('ts-in-progress', stats.in_progress || 0);
      el('ts-completed', stats.completed || 0);
      el('ts-failed', stats.failed || 0);
      el('ts-cancelled', stats.cancelled || 0);
      var total = (stats.open||0) + (stats.assigned||0) + (stats.in_progress||0) + (stats.completed||0) + (stats.failed||0) + (stats.cancelled||0);
      document.getElementById('task-meta').textContent = total + ' tasks';

      // Tokenomics from task stats
      el('tok-stake', (stats.total_stake_wei || 0).toLocaleString());
      el('tok-reward', (stats.total_reward_wei || 0).toLocaleString());
      var ratio = stats.total_stake_wei > 0 ? ((stats.total_reward_wei || 0) / stats.total_stake_wei * 100).toFixed(1) + '%' : '—';
      el('tok-ratio', ratio);
    }
  } catch(e) {
    // Task endpoint may not exist yet — use stats endpoint for tokenomics
    document.getElementById('task-meta').textContent = 'no task system';
  }

  // Tokenomics from knowledge entries (confirmations/challenges)
  try {
    var statsRes2 = await api('/stats');
    var d = statsRes2.data;
    if (d && d.insights) {
      var el = function(id, v) { var e = document.getElementById(id); if (e) e.textContent = v; };
      el('tok-confirms', d.insights.confirmed || 0);
      el('tok-challenges', d.insights.challenged || 0);
      var total = d.insights.total || 1;
      el('tok-chall-rate', ((d.insights.challenged || 0) / total * 100).toFixed(1) + '%');
      el('tok-avg-conf', ((d.insights.confirmed || 0) / total).toFixed(2));
    }
  } catch(e) {}

  // Task list
  try {
    var res = await api('/tasks?limit=20&sort=created_at&order=desc');
    var data = res.data;
    var tasks = data && data.items ? data.items : (Array.isArray(data) ? data : []);
    var hash = tasks.map(function(t) { return t.id + ':' + t.state; }).join('|');
    if (hash === taskHash) return;
    taskHash = hash;

    var tbody = document.getElementById('task-tbody');
    if (!tbody) return;

    // Clear placeholder row (has colspan, not 8 separate tds)
    if (tbody.children.length > 0 && tbody.children[0].children.length !== 8) {
      tbody.innerHTML = '';
    }

    while (tbody.children.length > tasks.length) tbody.removeChild(tbody.lastChild);
    while (tbody.children.length < tasks.length) {
      var tr = document.createElement('tr');
      for (var c = 0; c < 8; c++) tr.appendChild(document.createElement('td'));
      tbody.appendChild(tr);
    }
    if (tasks.length === 0) {
      tbody.innerHTML = '<tr><td colspan="8" style="color:var(--text-faint)">no tasks</td></tr>';
      return;
    }
    var stateColors = {
      open: 'var(--accent)', assigned: 'var(--cyan)', in_progress: 'var(--yellow)',
      completed: 'var(--green)', failed: 'var(--red)', cancelled: 'var(--text-faint)'
    };
    var prioColors = { critical: 'var(--red)', high: 'var(--orange)', medium: 'var(--yellow)', low: 'var(--text-dim)' };
    var now = Date.now() / 1000;
    for (var i = 0; i < tasks.length; i++) {
      var t = tasks[i];
      var row = tbody.children[i];
      var cells = row.children;
      cells[0].textContent = '#' + t.id;
      cells[0].style.cssText = 'font-weight:600;color:var(--text)';
      cells[1].textContent = (t.title || '').slice(0, 40);
      cells[2].textContent = t.kind || '—';
      cells[3].textContent = (t.priority || 'medium').toUpperCase();
      cells[3].style.color = prioColors[t.priority] || 'var(--text-dim)';
      cells[4].textContent = (t.state || 'open').toUpperCase();
      cells[4].style.color = stateColors[t.state] || 'var(--text)';
      cells[5].textContent = t.assignee || '—';
      cells[5].style.color = t.assignee ? 'var(--accent-bright)' : 'var(--text-faint)';
      cells[6].textContent = t.stake_wei ? (t.stake_wei / 1e18).toFixed(4) : '0';
      var age = t.created_at ? Math.max(0, now - t.created_at) : 0;
      cells[7].textContent = age < 60 ? Math.round(age) + 's' : age < 3600 ? Math.round(age/60) + 'm' : Math.round(age/3600) + 'h';
    }
  } catch(e) { /* tasks endpoint may not exist yet */ }
}
