/* ================================================================
   CHARTS — sparklines, heatmap, growth timeline, hero update,
            renderBlocks, renderAgent, renderLog, metricTick
   ================================================================ */

import { state } from './state.js';
import { rpc, fmtTs, shortHash } from './api.js';
import { resizeCanvas } from './pheromones.js';

/* ---------- Sparklines ---------- */
var sparkSizeCache = new WeakMap();
export function sparkDraw(canvas, data, color) {
  if (!canvas || !data) return;
  var dpr = window.devicePixelRatio || 1;
  var cached = sparkSizeCache.get(canvas);
  var cw = canvas.clientWidth, ch = canvas.clientHeight;
  if (!cached || cached.cw !== cw || cached.ch !== ch) {
    canvas.width = cw * dpr; canvas.height = ch * dpr;
    sparkSizeCache.set(canvas, {cw: cw, ch: ch});
  }
  var ctx = canvas.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  var w = cw, h = ch;
  ctx.clearRect(0,0,w,h);
  if (data.length < 2) return;
  var max = Math.max.apply(null, data.concat([1]));
  var min = Math.min.apply(null, data.concat([0]));
  var range = max - min || 1;
  ctx.strokeStyle = color; ctx.lineWidth = 1.5; ctx.lineJoin = 'round';
  ctx.beginPath();
  for (var i=0; i<data.length; i++) {
    var x = (i/(data.length-1)) * w;
    var y = h - ((data[i]-min)/range) * h;
    if (i===0) ctx.moveTo(x,y); else ctx.lineTo(x,y);
  }
  ctx.stroke();
  // Filled area
  ctx.lineTo(w,h); ctx.lineTo(0,h); ctx.closePath();
  ctx.fillStyle = color + '18'; ctx.fill();
}

/* ---------- pushSeries ---------- */
export function pushSeries(key, v) { var s = state.series[key]; s.push(v); if (s.length > 60) s.shift(); }

/* ---------- blocksPerSec ---------- */
export function blocksPerSec() {
  var last = state.blocks.slice(-10);
  if (last.length < 2) return 0;
  var dt = (last[last.length - 1].timestamp - last[0].timestamp);
  return dt > 0 ? (last.length - 1) / dt : 0;
}

/* ---------- Hero update ---------- */
export function updateHero() {
  var latest = state.blocks[state.blocks.length - 1];
  if (latest) {
    document.getElementById('h-block').textContent = latest.number.toLocaleString();
    document.getElementById('h-block-delta').textContent = '+' + blocksPerSec().toFixed(2) + ' bps';
    document.getElementById('h-fee').textContent = latest.baseFeeGwei.toFixed(2);
    document.getElementById('h-sat').textContent = latest.saturation.toFixed(1);
    var rec = state.blocks.slice(-5);
    if (rec.length >= 2) {
      var delta = rec[rec.length-1].baseFeeGwei - rec[0].baseFeeGwei;
      var el = document.getElementById('h-fee-delta');
      el.textContent = (delta >= 0 ? '+' : '') + delta.toFixed(2) + ' gwei';
      el.className = 'hero-delta ' + (delta > 0.5 ? 'up' : delta < -0.5 ? 'down' : 'neutral');
    }
    document.getElementById('h-sat-delta').textContent = latest.saturation > 80 ? 'congested' : latest.saturation > 50 ? 'moderate' : 'healthy';
    document.getElementById('h-sat-delta').className = 'hero-delta ' + (latest.saturation > 80 ? 'down' : 'up');
  }
}

/* ---------- Metric tick (1Hz) ---------- */
export async function metricTick() {
  var latest = state.blocks[state.blocks.length - 1];
  var rps = state.rpc.total - state.rpc.prev; state.rpc.prev = state.rpc.total;
  var cacheHit = 100; var cacheEntries = 0; var dirty = 0;
  try {
    var res = await rpc('mirage_status', []);
    var s = res.result;
    if (s) {
      cacheHit = (s.cache_hit_rate || s.cacheHitRate || 1) * 100;
      cacheEntries = s.cache_entries || s.cacheEntries || 0;
      dirty = s.dirty_slots || s.dirtySlots || 0;
    }
  } catch(e) {}

  var hInsights = state.chainInsightsTotal != null ? state.chainInsightsTotal : state.insights.size;
  var hPhero = state.chainPheromonesTotal != null ? state.chainPheromonesTotal : state.pheromones.length;

  pushSeries('block', latest ? latest.number : 0);
  pushSeries('fee', latest ? latest.baseFeeGwei : 0);
  pushSeries('sat', latest ? latest.saturation : 0);
  pushSeries('insights', hInsights);
  pushSeries('phero', hPhero);
  pushSeries('cache', cacheHit);
  pushSeries('rpc', rps);

  state.growthSeries.push({t: Date.now(), insights: hInsights, pheromones: hPhero, confirms: state.confirmsCount, challenges: state.challengesCount});
  if (state.growthSeries.length > 60) state.growthSeries.shift();

  var hCache = document.getElementById('h-cache');
  if (hCache) hCache.textContent = cacheHit.toFixed(1);
  var hCacheDelta = document.getElementById('h-cache-delta');
  if (hCacheDelta) hCacheDelta.textContent = cacheHit > 90 ? 'healthy' : 'degraded';

  document.getElementById('m-rpc').textContent = rps;
  document.getElementById('m-rpc-total').textContent = state.rpc.total;
  document.getElementById('m-fee').textContent = (latest ? latest.baseFeeGwei : 0).toFixed(2);
  document.getElementById('m-sat').textContent = (latest ? latest.saturation : 0).toFixed(1);
  document.getElementById('m-cache').textContent = cacheHit.toFixed(0);
  document.getElementById('m-cache-entries').textContent = cacheEntries.toLocaleString() + ' entries';
  document.getElementById('m-cache-dirty').textContent = dirty + ' dirty';
  var fees = state.blocks.map(function(b) { return b.baseFeeGwei; }).filter(function(x) { return x > 0; });
  if (fees.length) {
    document.getElementById('m-fee-range').textContent = Math.min.apply(null, fees).toFixed(1) + ' lo / ' + Math.max.apply(null, fees).toFixed(1) + ' hi';
  }
  var sats = state.blocks.map(function(b) { return b.saturation; }).filter(function(x) { return x > 0; });
  if (sats.length) {
    var avg = sats.reduce(function(a,b) { return a+b; }, 0) / sats.length;
    document.getElementById('m-sat-avg').textContent = 'avg ' + avg.toFixed(1) + '%';
    document.getElementById('m-sat-max').textContent = 'max ' + Math.max.apply(null, sats).toFixed(1) + '%';
  }
  var nowT = Date.now();
  state.postsLastMin = state.postsLastMin.filter(function(t) { return nowT - t < 60000; });
  document.getElementById('m-posts').textContent = state.postsLastMin.length;
  pushSeries('posts', state.postsLastMin.length);
  document.getElementById('m-authors').textContent = state.seenAuthors.size + ' authors';

  document.querySelectorAll('[data-spark]').forEach(function(c) {
    var key = c.dataset.spark;
    var colors = {block:'#a594ff',fee:'#fb923c',sat:'#f87171',insights:'#7c6ff7',phero:'#fb923c',cache:'#4ade80'};
    sparkDraw(c, state.series[key], colors[key]);
  });
  document.querySelectorAll('[data-metric]').forEach(function(c) {
    var key = c.dataset.metric;
    var colors = {rpc:'#22d3ee',search:'#a594ff',fee:'#fb923c',sat:'#f87171',cache:'#4ade80',posts:'#f472b6'};
    sparkDraw(c, state.series[key], colors[key]);
  });
  drawGrowth();
}

/* ---------- Block rendering (incremental) ---------- */
var lastBlockCount = 0;
export function renderBlocks() {
  var el = document.getElementById('block-stream');
  var blocks = state.blocks;
  var bps = blocksPerSec();
  document.getElementById('block-meta').textContent = blocks.length + ' · ' + bps.toFixed(1) + ' bps';

  // Only rebuild if new blocks arrived
  if (blocks.length === lastBlockCount) return;
  var newCount = blocks.length - lastBlockCount;
  lastBlockCount = blocks.length;

  // Add new blocks at top
  for (var n = 0; n < newCount && n < 5; n++) {
    var idx = blocks.length - 1 - n;
    if (idx < 0) break;
    var b = blocks[idx];
    var row = document.createElement('div');
    row.className = 'block-row fresh';
    row.dataset.blockNum = b.number;
    (function(bNum) {
      row.onclick = function() { state.selectedBlock = (state.selectedBlock === bNum) ? null : bNum; };
    })(b.number);
    var satColor = b.saturation > 90 ? 'var(--red)' : b.saturation > 60 ? 'var(--yellow)' : 'var(--green)';
    row.innerHTML =
      '<div class="block-head">' +
        '<span class="block-num">#' + b.number.toLocaleString() + '</span>' +
        '<span class="block-hash mono">' + shortHash(b.hash) + '</span>' +
      '</div>' +
      '<div class="block-metrics">' +
        '<span>tx <span class="tx">' + b.txCount + '</span></span>' +
        '<span>gas <span class="gas">' + (b.gasUsed/1e6).toFixed(1) + 'M</span></span>' +
        '<span>fee <span class="fee">' + b.baseFeeGwei.toFixed(1) + '</span></span>' +
      '</div>' +
      '<div class="sat-bar"><div style="width:' + b.saturation + '%; background:' + satColor + '"></div></div>';
    el.insertBefore(row, el.firstChild);
  }
  // Trim excess (keep max 40)
  while (el.children.length > 40) el.removeChild(el.lastChild);
}

/* ---------- Agent log (incremental) ---------- */
var lastAgentLogLen = 0;
export function renderAgent() {
  var el = document.getElementById('agent-log');
  var logs = state.agentLog;
  var agentCount = Math.max(state.seenAuthors.size, state.topoNodes.length);
  document.getElementById('agent-meta').textContent = agentCount + ' agents · ' + logs.length + ' events';
  document.getElementById('agents-chip').innerHTML = '<span class="dot"></span>' + agentCount + ' agents';

  if (logs.length === lastAgentLogLen) return;
  var newCount = logs.length - lastAgentLogLen;
  lastAgentLogLen = logs.length;

  for (var n = 0; n < newCount && n < 10; n++) {
    var idx = logs.length - 1 - n;
    if (idx < 0) break;
    var e = logs[idx];
    var row = document.createElement('div');
    row.className = 'agent-entry ' + e.type;
    row.innerHTML = '<span class="ts mono">' + fmtTs(e.ts) + '</span> <span class="author">' + e.author + '</span><span class="msg">' + e.msg + '</span>';
    el.insertBefore(row, el.firstChild);
  }
  while (el.children.length > 40) el.removeChild(el.lastChild);
}

/* ---------- Request log (incremental) ---------- */
var lastReqLogLen = 0;
export function renderLog() {
  var el = document.getElementById('log-view');
  var logs = state.requestLog;

  if (logs.length === lastReqLogLen) return;
  var newCount = logs.length - lastReqLogLen;
  lastReqLogLen = logs.length;

  for (var n = 0; n < newCount && n < 10; n++) {
    var idx = logs.length - 1 - n;
    if (idx < 0) break;
    var r = logs[idx];
    var row = document.createElement('div');
    row.className = 'log-line';
    row.innerHTML = '<span class="ts">' + fmtTs(r.ts) + '</span><span class="lv ' + r.lv + '">' + r.lv.toUpperCase() + '</span><span class="msg">' + r.msg + '</span>';
    el.insertBefore(row, el.firstChild);
  }
  while (el.children.length > 40) el.removeChild(el.lastChild);
}

/* ---------- Growth timeline ---------- */
var growthW = 0, growthH = 0;

export function drawGrowth() {
  var growthCanvas = document.getElementById('growth-canvas');
  var growthCtx = growthCanvas.getContext('2d');
  var w = growthCanvas.clientWidth, h = growthCanvas.clientHeight;
  if (growthW !== w || growthH !== h) { var d = resizeCanvas(growthCanvas); growthW = d.w; growthH = d.h; }
  growthCtx.clearRect(0, 0, w, h);
  var series = state.growthSeries;
  if (series.length < 2) {
    growthCtx.fillStyle = 'rgba(79,83,112,0.4)'; growthCtx.font = '11px monospace'; growthCtx.textAlign = 'center';
    growthCtx.fillText('accumulating data…', w/2, h/2);
    return;
  }
  var maxI = 1, maxP = 1;
  for (var i=0; i<series.length; i++) {
    if (series[i].insights > maxI) maxI = series[i].insights;
    if (series[i].pheromones > maxP) maxP = series[i].pheromones;
  }
  var maxVal = Math.max(maxI, maxP, 1);
  // Grid
  growthCtx.strokeStyle = 'rgba(79,83,112,0.08)'; growthCtx.lineWidth = 0.5;
  for (var gy = 0; gy < h; gy += 30) { growthCtx.beginPath(); growthCtx.moveTo(0,gy); growthCtx.lineTo(w,gy); growthCtx.stroke(); }
  // Plot helper
  var plot = function(key, color, fill) {
    growthCtx.strokeStyle = color; growthCtx.lineWidth = 1.5; growthCtx.lineJoin = 'round';
    growthCtx.beginPath();
    for (var i=0; i<series.length; i++) {
      var x = (i/(series.length-1)) * w;
      var y = h - (series[i][key]/maxVal) * h * 0.85 - 5;
      if (i===0) growthCtx.moveTo(x,y); else growthCtx.lineTo(x,y);
    }
    growthCtx.stroke();
    if (fill) {
      growthCtx.lineTo(w, h); growthCtx.lineTo(0, h); growthCtx.closePath();
      growthCtx.fillStyle = color + '22'; growthCtx.fill();
    }
  };
  plot('insights', '#a594ff', true);
  plot('pheromones', '#fb923c', false);
  plot('confirms', '#4ade80', false);
  plot('challenges', '#f87171', false);
  document.getElementById('growth-i').textContent = series[series.length-1].insights;
  document.getElementById('growth-p').textContent = series[series.length-1].pheromones;
  document.getElementById('growth-c').textContent = series[series.length-1].confirms;
  document.getElementById('growth-ch').textContent = series[series.length-1].challenges;
}

/* ---------- Heatmap ---------- */
var heatmapW = 0, heatmapH = 0;

export function drawHeatmap() {
  var heatmapCanvas = document.getElementById('heatmap-canvas');
  var heatmapCtx = heatmapCanvas.getContext('2d');
  var w = heatmapCanvas.clientWidth, h = heatmapCanvas.clientHeight;
  if (heatmapW !== w || heatmapH !== h) { var d = resizeCanvas(heatmapCanvas); heatmapW = d.w; heatmapH = d.h; }
  heatmapCtx.clearRect(0, 0, w, h);
  var buckets = state.heatmapBuckets;
  if (!buckets || buckets.length === 0) {
    heatmapCtx.fillStyle = 'rgba(79,83,112,0.4)';
    heatmapCtx.font = '11px monospace';
    heatmapCtx.textAlign = 'center';
    heatmapCtx.fillText('waiting for heatmap data…', w / 2, h / 2);
    return;
  }
  // Find max per-bucket total for scaling
  var maxVal = 1;
  for (var i = 0; i < buckets.length; i++) {
    var total = (buckets[i].threat || 0) + (buckets[i].opportunity || 0) + (buckets[i].wisdom || 0);
    if (total > maxVal) maxVal = total;
  }
  var barW = Math.max(4, (w - 20) / buckets.length - 2);
  var gap = 2;
  var baseX = 10;
  // Stacked bars
  for (var i = 0; i < buckets.length; i++) {
    var b = buckets[i];
    var x = baseX + i * (barW + gap);
    var tVal = b.threat || 0, oVal = b.opportunity || 0, wVal = b.wisdom || 0;
    var total = tVal + oVal + wVal;
    var barH = (total / maxVal) * (h - 30);
    var y = h - 15 - barH;
    // Wisdom (bottom)
    var wH = total > 0 ? (wVal / total) * barH : 0;
    heatmapCtx.fillStyle = 'rgba(251,191,36,0.6)';
    heatmapCtx.fillRect(x, h - 15 - wH, barW, wH);
    // Opportunity (middle)
    var oH = total > 0 ? (oVal / total) * barH : 0;
    heatmapCtx.fillStyle = 'rgba(74,222,128,0.6)';
    heatmapCtx.fillRect(x, h - 15 - wH - oH, barW, oH);
    // Threat (top)
    var tH = total > 0 ? (tVal / total) * barH : 0;
    heatmapCtx.fillStyle = 'rgba(248,113,113,0.6)';
    heatmapCtx.fillRect(x, h - 15 - wH - oH - tH, barW, tH);
    // Time label (every nth bucket)
    if (i % Math.max(1, Math.floor(buckets.length / 6)) === 0 && b.time) {
      heatmapCtx.fillStyle = 'rgba(139,143,168,0.4)';
      heatmapCtx.font = '8px monospace'; heatmapCtx.textAlign = 'center';
      var tLabel = new Date(b.time * 1000).toLocaleTimeString([], {hour:'2-digit',minute:'2-digit'});
      heatmapCtx.fillText(tLabel, x + barW/2, h - 3);
    }
  }
}

/* Reset dimensions on resize (called from main.js) */
export function resetGrowthSize() { growthW = 0; growthH = 0; }
export function resetHeatmapSize() { heatmapW = 0; heatmapH = 0; }
