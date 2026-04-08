/* ================================================================
   AGENT TOPOLOGY — force-directed agent network graph
   ================================================================ */

import { state } from './state.js';
import { resizeCanvas } from './pheromones.js';

var topoW = 0, topoH = 0;

export function drawTopology(dt) {
  var topoCanvas = document.getElementById('topo-canvas');
  var topoCtx = topoCanvas.getContext('2d');
  var nodes = state.topoNodes;
  var edges = state.topoEdges;
  var w = topoCanvas.clientWidth, h = topoCanvas.clientHeight;
  if (topoW !== w || topoH !== h) { var d = resizeCanvas(topoCanvas); topoW = d.w; topoH = d.h; }
  topoCtx.clearRect(0, 0, w, h);
  // Grid
  topoCtx.strokeStyle = 'rgba(79,83,112,0.06)'; topoCtx.lineWidth = 0.5;
  for (var gx = 0; gx < w; gx += 50) { topoCtx.beginPath(); topoCtx.moveTo(gx,0); topoCtx.lineTo(gx,h); topoCtx.stroke(); }
  for (var gy = 0; gy < h; gy += 50) { topoCtx.beginPath(); topoCtx.moveTo(0,gy); topoCtx.lineTo(w,gy); topoCtx.stroke(); }

  if (!nodes.length) {
    topoCtx.fillStyle = 'rgba(79,83,112,0.4)';
    topoCtx.font = '11px monospace'; topoCtx.textAlign = 'center';
    topoCtx.fillText('waiting for agent topology…', w / 2, h / 2);
    return;
  }

  // Physics: repulsion between all nodes
  for (var i = 0; i < nodes.length; i++) {
    var n = nodes[i];
    n.vx *= 0.82; n.vy *= 0.82;
    for (var j = 0; j < nodes.length; j++) {
      if (i === j) continue;
      var m = nodes[j];
      var dx = n.x - m.x, dy = n.y - m.y, d2 = dx * dx + dy * dy + 1;
      var f = 3000 / d2;
      var dist = Math.sqrt(d2);
      n.vx += (dx / dist) * f * 0.02;
      n.vy += (dy / dist) * f * 0.02;
    }
    // Center gravity
    n.vx += (w / 2 - n.x) * 0.002;
    n.vy += (h / 2 - n.y) * 0.002;
  }
  // Spring forces
  for (var ei = 0; ei < edges.length; ei++) {
    var e = edges[ei];
    var a = nodes.find(function(n) { return n.id === e.from; });
    var b = nodes.find(function(n) { return n.id === e.to; });
    if (!a || !b) continue;
    var dx = b.x - a.x, dy = b.y - a.y, dist = Math.sqrt(dx * dx + dy * dy) + 1;
    var idealDist = 120;
    var force = (dist - idealDist) * 0.005;
    a.vx += (dx / dist) * force; a.vy += (dy / dist) * force;
    b.vx -= (dx / dist) * force; b.vy -= (dy / dist) * force;
  }
  // Integrate
  for (var i = 0; i < nodes.length; i++) {
    var n = nodes[i];
    n.x += n.vx; n.y += n.vy;
    n.x = Math.max(30, Math.min(w - 30, n.x));
    n.y = Math.max(30, Math.min(h - 30, n.y));
  }
  // Draw edges
  for (var ei = 0; ei < edges.length; ei++) {
    var e = edges[ei];
    var a = nodes.find(function(n) { return n.id === e.from; });
    var b = nodes.find(function(n) { return n.id === e.to; });
    if (!a || !b) continue;
    var alpha = Math.min(0.5, e.weight * 0.15);
    var edgeColor = e.type === 'confirmed' ? '74,222,128' : e.type === 'challenged' ? '248,113,113' : '124,111,247';
    topoCtx.strokeStyle = 'rgba(' + edgeColor + ',' + alpha + ')';
    topoCtx.lineWidth = 1 + e.weight * 0.3;
    topoCtx.beginPath(); topoCtx.moveTo(a.x, a.y); topoCtx.lineTo(b.x, b.y); topoCtx.stroke();
    // Weight label
    if (e.weight > 2) {
      var mx = (a.x + b.x) / 2, my = (a.y + b.y) / 2;
      topoCtx.fillStyle = 'rgba(' + edgeColor + ',' + (alpha * 0.8) + ')';
      topoCtx.font = '8px monospace'; topoCtx.textAlign = 'center';
      topoCtx.fillText(e.weight, mx, my - 3);
    }
  }
  // Draw nodes
  for (var i = 0; i < nodes.length; i++) {
    var n = nodes[i];
    var r = 8 + Math.min(12, (n.insightsPosted + n.confirmationsGiven) * 0.5);
    var roleColors = {
      researcher: {r:165,g:148,b:255}, validator: {r:74,g:222,b:128},
      challenger: {r:248,g:113,b:113}, synthesizer: {r:34,g:211,b:238},
    };
    var c = roleColors[n.role] || {r:124,g:111,b:247};
    // Glow
    var gr = topoCtx.createRadialGradient(n.x, n.y, 0, n.x, n.y, r * 2.5);
    gr.addColorStop(0, 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',0.25)');
    gr.addColorStop(1, 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',0)');
    topoCtx.fillStyle = gr; topoCtx.beginPath(); topoCtx.arc(n.x, n.y, r * 2.5, 0, Math.PI * 2); topoCtx.fill();
    // Node
    topoCtx.fillStyle = 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',0.7)';
    topoCtx.beginPath(); topoCtx.arc(n.x, n.y, r, 0, Math.PI * 2); topoCtx.fill();
    // Label
    topoCtx.fillStyle = 'rgba(230,232,240,0.8)'; topoCtx.font = '9px monospace'; topoCtx.textAlign = 'center';
    topoCtx.fillText(n.id.slice(0, 12), n.x, n.y - r - 5);
    // Role badge
    topoCtx.fillStyle = 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',0.5)';
    topoCtx.font = '7px monospace';
    topoCtx.fillText(n.role || 'agent', n.x, n.y + r + 10);
    // Stats
    if (n.insightsPosted > 0 || n.confirmationsGiven > 0) {
      topoCtx.font = '7px monospace';
      topoCtx.fillStyle = 'rgba(165,148,255,0.6)';
      topoCtx.fillText('i:' + n.insightsPosted, n.x - 12, n.y + 3);
      topoCtx.fillStyle = 'rgba(74,222,128,0.6)';
      topoCtx.fillText('c:' + n.confirmationsGiven, n.x + 8, n.y + 3);
    }
  }
  // Legend
  topoCtx.font = '9px monospace'; topoCtx.textAlign = 'left';
  var ly = h - 50;
  var roles = [['researcher','165,148,255'],['validator','74,222,128'],['challenger','248,113,113'],['synthesizer','34,211,238']];
  for (var ri = 0; ri < roles.length; ri++) {
    topoCtx.fillStyle = 'rgba(' + roles[ri][1] + ',0.6)';
    topoCtx.beginPath(); topoCtx.arc(12, ly, 3, 0, Math.PI*2); topoCtx.fill();
    topoCtx.fillStyle = 'rgba(139,143,168,0.5)';
    topoCtx.fillText(roles[ri][0], 22, ly + 3);
    ly += 12;
  }
}

/* Reset dimensions on resize (called from main.js) */
export function resetTopoSize() { topoW = 0; topoH = 0; }
