/* ================================================================
   MAIN — init, connect(), frame(), event wiring, interval setup
   ================================================================ */

import { state } from './state.js';
import { rpc, api, apiPost, logReq, logAgent, toast, onRenderLog, onRenderAgent } from './api.js';
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
        document.getElementById('fork-chip').innerHTML = '<span class="dot"></span>FORK: ' + (fb ? fb.toLocaleString() : '?');
        document.getElementById('fork-chip').className = fb ? 'chip ok' : 'chip';
        var fu = s.forkUrl || s.fork_url;
        if (fu) document.getElementById('fork-chip').title = fu;
      }
    } catch(e2) {}
    // Initial data fetch
    await Promise.allSettled([pollBlock(), pollChain(), pollEntries(), pollEdges(), pollKinds(), pollPheroSummary(), pollHeatmap(), pollTopology()]);
    // Clear existing intervals
    if (state.pollers.blocks) clearInterval(state.pollers.blocks);
    if (state.pollers.chain) clearInterval(state.pollers.chain);
    if (state.pollers.heatmap) clearInterval(state.pollers.heatmap);
    if (state.pollers.topo) clearInterval(state.pollers.topo);
    if (state.pollers.kinds) clearInterval(state.pollers.kinds);
    if (state.pollers.edges) clearInterval(state.pollers.edges);
    if (state.pollers.entries) clearInterval(state.pollers.entries);
    if (state.pollers.summary) clearInterval(state.pollers.summary);
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
    pollAgentRegistry();
    pollLeaderboard();
    pollTasks();
    // Auto-connect WebSocket for real-time updates
    if (!state.wsLive) toggleWs();
  } catch (e) {
    state.connected = false;
    chip.className = 'chip err';
    document.getElementById('conn-label').textContent = 'OFFLINE';
    logReq('err', 'not connected: ' + e.message + ' · retrying in 5s');
    setTimeout(connect, 5000);
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
document.getElementById('btn-reconnect').onclick = function() { state.rpcUrl = document.getElementById('rpc-url').value.trim(); connect(); };
document.getElementById('btn-ws').onclick = toggleWs;
document.getElementById('btn-clear').onclick = function() {
  state.blocks = []; state.insights.clear(); state.pheromones.length = 0;
  graphNodes.length = 0; graphEdges.length = 0; hdcHighlights.clear();
  state.selectedNode = null; state.confirmsCount = 0; state.challengesCount = 0;
  state.agentLog = []; state.growthSeries = []; state.seenAuthors.clear();
  state.topoNodes = []; state.topoEdges = []; state.heatmapBuckets = [];
  renderBlocks(); renderAgent(); renderDetail(null);
  toast('info', 'cleared');
};
document.getElementById('btn-clear-log').onclick = function() { state.requestLog = []; renderLog(); };
document.getElementById('ph-intensity').oninput = function(e) { document.getElementById('ph-int-label').textContent = (e.target.value/100).toFixed(2); };

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

/* ---------- Register Agent ---------- */
document.getElementById('btn-register').onclick = async function() {
  var id = document.getElementById('reg-id').value.trim();
  var role = document.getElementById('reg-role').value.trim();
  if (!id) { toast('warn', 'enter an agent ID'); return; }
  try {
    var res = await apiPost('/agents', { id: id, pubkey: [], role: role });
    if (res.data && res.data.ok) {
      toast('ok', 'registered agent: ' + id);
      logAgent('act', id, 'registered as ' + role);
      pollAgentRegistry();
    } else {
      toast('warn', (res.data && res.data.error) || 'registration failed');
    }
  } catch(e) { toast('err', e.message); }
};

/* ---------- Boot ---------- */
logReq('info', 'dashboard init · connecting to real mirage fork · REST API + JSON-RPC');
connect();
setInterval(metricTick, 1000);
window.addEventListener('resize', function() {
  resetPheroSize(); resetGraphSize(); resetGrowthSize(); resetHeatmapSize(); resetTopoSize();
});
