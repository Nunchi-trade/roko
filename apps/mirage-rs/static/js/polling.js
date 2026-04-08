/* ================================================================
   POLLING — all poll* functions (block, chain, entries, edges,
             kinds, pheroSummary, heatmap, topology)
   ================================================================ */

import { state } from './state.js';
import { rpc, api, logAgent, parseHexU64, parseHexBig, weiToGwei, fmtTs } from './api.js';
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
    var searchRes = await api('/knowledge/search?q=' + encodeURIComponent(sq) + '&k=30');
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
    var res = await api('/knowledge/entries?limit=200&sort=created_at&order=desc');
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
        x: old ? old.x : w/2 + (Math.random()-0.5)*w*0.6,
        y: old ? old.y : h/2 + (Math.random()-0.5)*h*0.6,
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
    // Update leaderboard table
    var sorted = state.topoNodes.slice().sort(function(a, b) {
      return (b.insightsPosted + b.confirmationsGiven) - (a.insightsPosted + a.confirmationsGiven);
    });
    var lb = document.getElementById('leaderboard-tbody');
    if (lb) {
      lb.innerHTML = '';
      for (var si = 0; si < Math.min(sorted.length, 10); si++) {
        var agent = sorted[si];
        var tr = document.createElement('tr');
        var roleColors = {
          researcher: 'var(--accent-bright)', validator: 'var(--green)',
          challenger: 'var(--red)', synthesizer: 'var(--cyan)',
        };
        tr.innerHTML =
          '<td style="color:' + (roleColors[agent.role] || 'var(--text)') + '">' + agent.id.slice(0,12) + '</td>' +
          '<td>' + (agent.role || 'agent') + '</td>' +
          '<td style="color:var(--accent-bright)">' + agent.insightsPosted + '</td>' +
          '<td style="color:var(--green)">' + agent.confirmationsGiven + '</td>' +
          '<td style="color:var(--red)">' + agent.challengesGiven + '</td>' +
          '<td style="font-weight:600">' + (agent.totalWeight || 0).toFixed(2) + '</td>';
        lb.appendChild(tr);
      }
    }
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

/* ---------- Poll agent registry via REST ---------- */
export async function pollAgentRegistry() {
  try {
    var res = await api('/agents');
    var data = res.data;
    var agents = data && data.items ? data.items : (Array.isArray(data) ? data : []);
    state.registeredAgents = agents;
    var tbody = document.getElementById('agent-reg-tbody');
    if (!tbody) return;
    tbody.innerHTML = '';
    if (agents.length === 0) {
      tbody.innerHTML = '<tr><td colspan="8" style="color:var(--text-faint)">no agents registered</td></tr>';
      document.getElementById('agent-reg-meta').textContent = '0 agents';
      return;
    }
    for (var i = 0; i < agents.length; i++) {
      var a = agents[i];
      var tr = document.createElement('tr');
      var alive = a.is_alive !== false;
      var statusClass = alive ? 'color:var(--green)' : 'color:var(--red)';
      var statusLabel = alive ? 'ALIVE' : 'OFFLINE';
      var lastSeen = a.last_heartbeat ? new Date(a.last_heartbeat * 1000).toLocaleTimeString('en-US', {hour12: false}) : '—';
      tr.innerHTML =
        '<td style="font-weight:600;color:var(--accent-bright)">' + (a.id || a.agent_id || '?') + '</td>' +
        '<td>' + (a.role || '—') + '</td>' +
        '<td style="' + statusClass + '">' + statusLabel + '</td>' +
        '<td>' + (a.tasks_completed || 0) + '</td>' +
        '<td>' + (a.tasks_failed || 0) + '</td>' +
        '<td>' + (a.total_tokens || 0).toLocaleString() + '</td>' +
        '<td>$' + (a.total_cost_usd || 0).toFixed(4) + '</td>' +
        '<td>' + lastSeen + '</td>';
      tbody.appendChild(tr);
    }
    document.getElementById('agent-reg-meta').textContent = agents.length + ' agents';
    // Update hero agents count
    document.getElementById('h-agents').textContent = agents.length;
    pushSeries('agents', agents.length);
  } catch (e) { /* ignore */ }
}

/* ---------- Poll leaderboard from topology ---------- */
export async function pollLeaderboard() {
  try {
    var res = await api('/agents/topology');
    var data = res.data;
    if (!data || !data.nodes) return;
    var nodes = data.nodes.slice().sort(function(a, b) {
      return (b.contribution_count || 0) - (a.contribution_count || 0);
    });
    var tbody = document.getElementById('leaderboard-tbody');
    if (!tbody) return;
    tbody.innerHTML = '';
    if (nodes.length === 0) {
      tbody.innerHTML = '<tr><td colspan="5" style="color:var(--text-faint)">no agents</td></tr>';
      return;
    }
    for (var i = 0; i < nodes.length; i++) {
      var n = nodes[i];
      var tr = document.createElement('tr');
      tr.innerHTML =
        '<td style="font-weight:600;color:var(--accent-bright)">' + n.id + '</td>' +
        '<td>' + (n.insightsPosted || n.contribution_count || 0) + '</td>' +
        '<td style="color:var(--green)">' + (n.confirmationsGiven || 0) + '</td>' +
        '<td style="color:var(--red)">' + (n.challengesGiven || 0) + '</td>' +
        '<td>' + (n.totalWeight || 0).toFixed(2) + '</td>';
      tbody.appendChild(tr);
    }
    document.getElementById('leaderboard-meta').textContent = nodes.length + ' agents';
  } catch (e) { /* ignore */ }
}
