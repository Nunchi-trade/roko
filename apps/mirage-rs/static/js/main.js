/* ================================================================
   MAIN — init, connect(), frame(), event wiring, interval setup
   ================================================================ */

import {
  getSelectedMergedAgent,
  listTransportOptionsForAgent,
  resetRegistryState,
  setRegistryMessageResult,
  setRegistryTransportPreference,
  setSelectedMergedAgent,
  state,
} from './state.js';
import {
  api,
  apiPost,
  circuitBreakerState,
  loadInitialRemoteBase,
  logReq,
  logAgent,
  onRenderAgent,
  onRenderLog,
  rpc,
  sendAgentMessage,
  setRemoteBase,
  toast,
} from './api.js';
import { pollBlock, pollChain, pollEntries, pollEdges, pollKinds, pollPheroSummary, pollHeatmap, pollTopology, pollAgentRegistry, pollLeaderboard, pollTasks } from './polling.js';
import { sparkDraw, drawGrowth, drawHeatmap, metricTick, renderBlocks, renderAgent, renderLog } from './charts.js';
import { drawPheromones, depositPheromoneParticle, resetPheroSize, handlePheroMouseMove, handlePheroClick, handlePheroMouseLeave, handlePheroFilterClick } from './pheromones.js';
import { drawGraph, canvasToNode, openDetail, renderDetail, addInsightNode, graphNodes, graphEdges, hdcHighlights, resetGraphSize } from './graph.js';
import { drawTopology, resetTopoSize } from './topology.js';
import { toggleWs } from './ws.js';
import { resetGrowthSize, resetHeatmapSize } from './charts.js';

/* ---------- Wire render callbacks (avoids circular deps) ---------- */
onRenderLog(renderLog);
onRenderAgent(renderAgent);

/* ---------- Remote base + registry composer ---------- */
function updateNetChip() {
  var chip = document.getElementById('net-chip');
  var label = document.getElementById('net-label');
  if (!chip || !label) return;
  label.textContent = state.remoteBase + ' · /api + /relay';
  chip.title = 'mirage base ' + state.remoteBase + ' · relay is derived as ' + state.relayBase;
}

function ensureRemoteBaseInput() {
  document.getElementById('rpc-url').value = state.remoteBase;
  updateNetChip();
}

function renderMessageComposer() {
  var select = document.getElementById('msg-agent');
  var path = document.getElementById('msg-path');
  var result = document.getElementById('msg-result');
  var note = document.getElementById('msg-note');
  if (!select || !path || !result || !note) return;

  var agent = getSelectedMergedAgent();
  var options = listTransportOptionsForAgent(agent);
  path.innerHTML = '';
  for (var i = 0; i < options.length; i++) {
    var option = document.createElement('option');
    option.value = options[i].value;
    option.textContent = options[i].label.toUpperCase();
    path.appendChild(option);
  }
  path.value = state.registry.transportPreference;
  if (path.value !== state.registry.transportPreference) {
    path.value = 'auto';
  }
  if (select.value !== state.registry.selectedAgentKey) {
    select.value = state.registry.selectedAgentKey || '';
  }

  if (!agent) {
    note.textContent = 'select a discovered agent to test direct or relay transport';
    result.textContent = 'waiting for discovery…';
    return;
  }

  var direct = agent.directEndpoint || 'no direct endpoint';
  var relay = agent.relayAvailable ? ('relay agent ' + (agent.relayAgentId || agent.agentId)) : 'relay unavailable';
  note.textContent = 'selected ' + (agent.displayName || agent.agentId) + ' · direct ' + direct + ' · ' + relay;

  if (!state.registry.messageResult) {
    result.textContent = 'ready · choose a path and send a prompt';
    return;
  }

  result.textContent = JSON.stringify(state.registry.messageResult, null, 2);
}

/* ---------- Animation loop ---------- */
var lastT = performance.now();
function frame() {
  var t = performance.now(); var dt = Math.min(100, t - lastT); lastT = t;
  drawPheromones(dt); drawGraph(dt); drawHeatmap(); drawTopology(dt);
  requestAnimationFrame(frame);
}
requestAnimationFrame(frame);

/* ---------- Connect ---------- */
async function connect() {
  var chip = document.getElementById('conn-chip');
  try {
    await rpc('eth_blockNumber');
    state.connected = true;
    chip.className = 'chip ok';
    document.getElementById('conn-label').textContent = 'CONNECTED';
    // Fork status
    try {
      var statusRes = await rpc('mirage_status', []);
      var s = statusRes.result;
      if (s) {
        var fb = s.forkBlock || s.fork_block || 0;
        state.forkBlock = fb || state.forkBlock;
        document.getElementById('fork-chip').innerHTML = '<span class="dot"></span>FORK: ' + (fb ? fb.toLocaleString() : '?');
        document.getElementById('fork-chip').className = fb ? 'chip ok' : 'chip';
        var fu = s.forkUrl || s.fork_url;
        if (fu) document.getElementById('fork-chip').title = fu;
      }
    } catch(e2) {}
    // Initial data fetch
    await Promise.allSettled([pollBlock(), pollChain(), pollEntries(), pollEdges(), pollKinds(), pollPheroSummary(), pollHeatmap(), pollTopology(), pollAgentRegistry()]);
    // Clear existing intervals
    if (state.pollers.blocks) clearInterval(state.pollers.blocks);
    if (state.pollers.chain) clearInterval(state.pollers.chain);
    if (state.pollers.heatmap) clearInterval(state.pollers.heatmap);
    if (state.pollers.topo) clearInterval(state.pollers.topo);
    if (state.pollers.kinds) clearInterval(state.pollers.kinds);
    if (state.pollers.edges) clearInterval(state.pollers.edges);
    if (state.pollers.entries) clearInterval(state.pollers.entries);
    if (state.pollers.summary) clearInterval(state.pollers.summary);
    if (state.pollers.agentReg) clearInterval(state.pollers.agentReg);
    if (state.pollers.leaderboard) clearInterval(state.pollers.leaderboard);
    if (state.pollers.tasks) clearInterval(state.pollers.tasks);
    // Start polling
    state.pollers.blocks = setInterval(pollBlock, 1000);
    state.pollers.chain = setInterval(pollChain, 2000);
    state.pollers.heatmap = setInterval(pollHeatmap, 10000);
    state.pollers.topo = setInterval(pollTopology, 5000);
    state.pollers.kinds = setInterval(pollKinds, 15000);
    state.pollers.edges = setInterval(pollEdges, 5000);
    state.pollers.entries = setInterval(pollEntries, 3000);
    state.pollers.summary = setInterval(pollPheroSummary, 5000);
    state.pollers.agentReg = setInterval(pollAgentRegistry, 5000);
    state.pollers.leaderboard = setInterval(pollLeaderboard, 8000);
    state.pollers.tasks = setInterval(pollTasks, 3000);
    pollLeaderboard();
    pollTasks();
    // Auto-connect WebSocket for real-time updates
    if (!state.wsLive) toggleWs();
  } catch (e) {
    state.connected = false;
    chip.className = 'chip err';
    var cb = circuitBreakerState();
    var retryDelay = cb.open ? Math.max(5000, cb.nextRetryAt - Date.now()) : 5000;
    var retrySec = Math.round(retryDelay / 1000);
    document.getElementById('conn-label').textContent = 'OFFLINE · retry ' + retrySec + 's';
    logReq('err', 'not connected: ' + e.message + ' · retrying in ' + retrySec + 's');
    setTimeout(connect, retryDelay);
  }
}

/* ---------- Graph canvas event wiring ---------- */
var graphCanvas = document.getElementById('graph-canvas');
graphCanvas.addEventListener('mousemove', function(ev) {
  var n = canvasToNode(ev);
  var tt = document.getElementById('node-tooltip');
  if (n) {
    state.hoveredNode = n.id;
    var ins = state.insights.get(n.id);
    tt.style.display = 'block';
    tt.style.left = (ev.offsetX + 14) + 'px';
    tt.style.top = (ev.offsetY + 14) + 'px';
    tt.innerHTML =
      '<div class="nt-kind">' + n.kind + '</div>' +
      '<div class="nt-content">' + (n.content || '(no content)') + '</div>' +
      '<div class="nt-meta">' +
        '<span>conf <b style="color:var(--green)">' + n.conf + '</b></span>' +
        '<span>chall <b style="color:var(--red)">' + n.chall + '</b></span>' +
        (ins ? '<span>w <b>' + (ins.weight||1).toFixed(2) + '</b></span>' : '') +
      '</div>';
    graphCanvas.style.cursor = 'pointer';
  } else {
    state.hoveredNode = null;
    tt.style.display = 'none';
    graphCanvas.style.cursor = 'default';
  }
});
graphCanvas.addEventListener('click', async function(ev) {
  var n = canvasToNode(ev);
  if (!n) { state.selectedNode = null; hdcHighlights.clear(); renderDetail(null); return; }
  state.selectedNode = n.id;
  await openDetail(n);
});

/* ---------- Topology canvas event wiring ---------- */
var topoCanvas = document.getElementById('topo-canvas');
topoCanvas.addEventListener('mousemove', function(ev) {
  var rect = topoCanvas.getBoundingClientRect();
  var mx = ev.clientX - rect.left, my = ev.clientY - rect.top;
  var found = null;
  for (var i = 0; i < state.topoNodes.length; i++) {
    var n = state.topoNodes[i];
    var dx = mx - n.x, dy = my - n.y;
    if (dx*dx + dy*dy < 20*20) { found = n; break; }
  }
  var tt = document.getElementById('topo-tooltip');
  if (found) {
    tt.style.display = 'block';
    tt.style.left = (ev.offsetX + 14) + 'px';
    tt.style.top = (ev.offsetY + 14) + 'px';
    tt.innerHTML =
      '<div class="nt-kind">' + found.id + '</div>' +
      '<div class="nt-content">role: ' + (found.role || 'agent') + '</div>' +
      '<div class="nt-meta">' +
      '<span>insights <b style="color:var(--accent-bright)">' + found.insightsPosted + '</b></span>' +
      '<span>confirmed <b style="color:var(--green)">' + found.confirmationsGiven + '</b></span>' +
      '<span>challenged <b style="color:var(--red)">' + found.challengesGiven + '</b></span>' +
      '<span>weight <b style="color:var(--text)">' + (found.totalWeight || 0).toFixed(2) + '</b></span>' +
      '</div>';
    topoCanvas.style.cursor = 'pointer';
  } else {
    tt.style.display = 'none';
    topoCanvas.style.cursor = 'default';
  }
});

/* ---------- Pheromone canvas event wiring ---------- */
var pheroCanvas = document.getElementById('phero-canvas');
pheroCanvas.addEventListener('mousemove', handlePheroMouseMove);
pheroCanvas.addEventListener('click', handlePheroClick);
pheroCanvas.addEventListener('mouseleave', handlePheroMouseLeave);
document.getElementById('phero-filters').addEventListener('click', handlePheroFilterClick);

/* ---------- UI wiring ---------- */
setRemoteBase(loadInitialRemoteBase());
ensureRemoteBaseInput();
window.addEventListener('registry-updated', renderMessageComposer);
document.getElementById('btn-reconnect').onclick = function() {
  setRemoteBase(document.getElementById('rpc-url').value.trim());
  updateNetChip();
  if (state.ws) {
    try { state.ws.close(); } catch (e) { /* ignore */ }
    state.ws = null;
    state.wsLive = false;
  }
  connect();
};
document.getElementById('btn-ws').onclick = toggleWs;
document.getElementById('btn-clear').onclick = function() {
  state.blocks = []; state.insights.clear(); state.pheromones.length = 0;
  graphNodes.length = 0; graphEdges.length = 0; hdcHighlights.clear();
  state.selectedNode = null; state.confirmsCount = 0; state.challengesCount = 0;
  state.agentLog = []; state.growthSeries = []; state.seenAuthors.clear();
  state.topoNodes = []; state.topoEdges = []; state.heatmapBuckets = [];
  resetRegistryState();
  renderBlocks(); renderAgent(); renderDetail(null);
  renderMessageComposer();
  pollAgentRegistry();
  toast('info', 'cleared');
};
document.getElementById('btn-clear-log').onclick = function() { state.requestLog = []; renderLog(); };
document.getElementById('ph-intensity').oninput = function(e) { document.getElementById('ph-int-label').textContent = (e.target.value/100).toFixed(2); };
document.getElementById('msg-agent').addEventListener('change', function(e) {
  setSelectedMergedAgent(e.target.value);
  setRegistryMessageResult(null);
  renderMessageComposer();
});
document.getElementById('msg-path').addEventListener('change', function(e) {
  setRegistryTransportPreference(e.target.value);
  renderMessageComposer();
});
document.getElementById('agent-reg-tbody').addEventListener('click', function(e) {
  var button = e.target.closest('button[data-agent-key]');
  if (!button) return;
  setSelectedMergedAgent(button.dataset.agentKey);
  setRegistryMessageResult(null);
  renderMessageComposer();
  document.getElementById('msg-prompt').focus();
});
document.getElementById('btn-send-agent').onclick = async function() {
  var agent = getSelectedMergedAgent();
  var prompt = document.getElementById('msg-prompt').value.trim();
  if (!agent) { toast('warn', 'select an agent'); return; }
  if (!prompt) { toast('warn', 'enter a prompt'); return; }
  try {
    var response = await sendAgentMessage(agent, prompt, state.registry.transportPreference);
    setRegistryMessageResult(response);
    renderMessageComposer();
    toast('ok', (response.transport || 'message') + ' response received');
    logAgent('act', agent.agentId, 'message via ' + response.transport + ' completed');
  } catch (e) {
    setRegistryMessageResult({ error: e.message });
    renderMessageComposer();
    toast('err', e.message);
  }
};

document.getElementById('btn-post').onclick = async function() {
  var kind = document.getElementById('ins-kind').value;
  var author = document.getElementById('ins-author').value;
  var content = document.getElementById('ins-content').value;
  if (!content) { toast('warn', 'enter content'); return; }
  try {
    var res = await apiPost('/knowledge/entries', {kind: kind, content: content, author: author, stake_wei: 0});
    var result = res.data;
    if (result) {
      toast('ok', result.outcome || 'posted');
      var id = (result.id || '').replace(/^insight:/, '');
      state.insights.set(id, {id: id, kind: kind, content: content, author: author, conf:0, chall:0, weight:1.0});
      state.seenAuthors.add(author);
      addInsightNode(id, kind, content);
      state.postsLastMin.push(Date.now());
      logAgent('observe', author, content.slice(0,80));
    }
  } catch(e) { toast('err', e.message); }
};
document.getElementById('btn-deposit').onclick = async function() {
  var kind = document.getElementById('ph-kind').value;
  var content = document.getElementById('ph-content').value;
  var intensity = parseFloat(document.getElementById('ph-intensity').value) / 100;
  if (!content) { toast('warn', 'enter content'); return; }
  try {
    var res = await apiPost('/pheromones', {kind: kind, content: content, intensity: intensity});
    var result = res.data;
    if (result) {
      toast('ok', kind + ' deposited');
      depositPheromoneParticle(kind, content, intensity, result.id);
      logAgent(kind, 'human-operator', content.slice(0,80));
    }
  } catch(e) { toast('err', e.message); }
};
document.getElementById('btn-decay').onclick = async function() {
  try {
    var res = await apiPost('/knowledge/decay', {});
    toast('ok', (res.data ? res.data.prunedCount || 0 : 0) + ' pruned');
  } catch(e) { toast('err', e.message); }
};
document.getElementById('btn-search').onclick = async function() {
  var query = document.getElementById('qu-query').value;
  var k = parseInt(document.getElementById('qu-k').value) || 8;
  var kind = document.getElementById('qu-kind').value || '';
  if (!query) { toast('warn', 'enter query'); return; }
  try {
    var path = '/knowledge/search?q=' + encodeURIComponent(query) + '&k=' + k;
    if (kind) path += '&kind=' + encodeURIComponent(kind);
    var res = await api(path);
    var result = res.data;
    if (!result || !result.results) return;
    hdcHighlights.clear();
    var highlighted = 0;
    var container = document.getElementById('search-results');
    container.innerHTML = '';
    for (var i = 0; i < result.results.length; i++) {
      var hit = result.results[i];
      var rid = (hit.id || '').replace(/^insight:/, '');
      if (rid && graphNodes.find(function(x) { return x.id === rid; })) {
        hdcHighlights.add(rid); highlighted++;
      }
      var div = document.createElement('div');
      div.className = 'sr-item';
      div.dataset.id = rid;
      var scorePct = Math.round((hit.score || 0) * 100);
      div.innerHTML =
        '<div class="sr-head">' +
          '<span class="sr-badge ' + (hit.kind || 'insight') + '">' + (hit.kind || '—') + '</span>' +
          '<span class="sr-score">' + (hit.similarity || 0).toFixed(3) + ' sim</span>' +
          '<span class="sr-bar"><span class="sr-bar-fill" style="width:' + scorePct + '%"></span></span>' +
          '<span class="sr-score" style="color:var(--text)">' + scorePct + '%</span>' +
        '</div>' +
        '<div class="sr-content">' + (hit.content || '').slice(0, 140) + '</div>' +
        '<div class="sr-meta">' +
          (hit.author || '?') + ' · ' + (hit.confirmations || 0) + ' conf · ' +
          (hit.state || '—') + ' · w=' + (hit.weight || 0).toFixed(2) +
        '</div>';
      div.onclick = (function(capturedId) {
        return function() {
          var gn = graphNodes.find(function(x) { return x.id === capturedId; });
          if (gn) { state.selectedNode = gn; openDetail(gn); }
        };
      })(rid);
      container.appendChild(div);
    }
    toast('info', result.results.length + ' hits · ' + highlighted + ' in graph · ' + Math.round(res.ms) + 'ms');
    logAgent('observe', 'search', 'query="' + query + '" → ' + result.results.length + ' hits');
  } catch(e) { toast('err', e.message); }
};

/* ---------- Boot ---------- */
renderMessageComposer();
logReq('info', 'dashboard init · remote mirage base + ERC-8004 identity + relay reachability');
connect();
setInterval(metricTick, 1000);
window.addEventListener('resize', function() {
  resetPheroSize(); resetGraphSize(); resetGrowthSize(); resetHeatmapSize(); resetTopoSize();
});
