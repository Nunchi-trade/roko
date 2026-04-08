/* ================================================================
   KNOWLEDGE GRAPH — force-directed insight visualization
   ================================================================ */

import { state } from './state.js';
import { api, apiPost, logAgent, toast } from './api.js';

var I_COLORS = {
  insight: {r:165,g:148,b:255}, heuristic: {r:34,g:211,b:238},
  warning: {r:251,g:191,b:36}, causal_link: {r:244,g:114,b:182},
  strategy_fragment: {r:96,g:165,b:250}, anti_knowledge: {r:248,g:113,b:113},
};
var graphW=0, graphH=0;
export var graphNodes = [];
export var graphEdges = [];
export var hdcHighlights = new Set();

export function addInsightNode(id, kind, content, opts) {
  if (!opts) opts = {};
  if (graphNodes.find(function(n) { return n.id === id; })) return;
  var graphCanvas = document.getElementById('graph-canvas');
  var w = graphCanvas.clientWidth, h = graphCanvas.clientHeight;
  graphNodes.push({
    id: id, kind: kind, content: content || '',
    x: w/2 + (Math.random()-0.5)*w*0.6, y: h/2 + (Math.random()-0.5)*h*0.6,
    vx: 0, vy: 0, radius: 6, pulse: 1, conf: 0, chall: 0, author: opts.author || null,
  });
  while (graphNodes.length > 400) graphNodes.shift();
  document.getElementById('graph-meta').textContent = graphNodes.length + ' nodes · ' + graphEdges.length + ' edges';
}

export function drawGraph(dt) {
  var graphCanvas = document.getElementById('graph-canvas');
  var graphCtx = graphCanvas.getContext('2d');
  var w = graphCanvas.clientWidth, h = graphCanvas.clientHeight;
  if (graphW !== w || graphH !== h) {
    var dpr = window.devicePixelRatio || 1;
    graphCanvas.width = w * dpr; graphCanvas.height = h * dpr;
    graphCtx = graphCanvas.getContext('2d'); graphCtx.scale(dpr, dpr);
    graphW = w; graphH = h;
  }
  graphCtx.clearRect(0,0,w,h);
  // Grid
  graphCtx.strokeStyle = 'rgba(79,83,112,0.06)'; graphCtx.lineWidth = 0.5;
  for (var gx=0; gx<w; gx+=60) { graphCtx.beginPath(); graphCtx.moveTo(gx,0); graphCtx.lineTo(gx,h); graphCtx.stroke(); }
  for (var gy=0; gy<h; gy+=60) { graphCtx.beginPath(); graphCtx.moveTo(0,gy); graphCtx.lineTo(w,gy); graphCtx.stroke(); }
  if (!graphNodes.length) {
    graphCtx.fillStyle='rgba(79,83,112,0.4)'; graphCtx.font='12px monospace'; graphCtx.textAlign='center';
    graphCtx.fillText('post or search insights to populate graph', w/2, h/2);
    return;
  }
  // Physics
  for (var i=0; i<graphNodes.length; i++) {
    var n = graphNodes[i];
    n.vx *= 0.88; n.vy *= 0.88;
    for (var j=0; j<graphNodes.length; j++) {
      if (i===j) continue;
      var m = graphNodes[j];
      var dx = n.x-m.x, dy = n.y-m.y, d2 = dx*dx+dy*dy+1;
      var f = 8000/d2;
      var dist = Math.sqrt(d2);
      n.vx += (dx/dist)*f*0.02; n.vy += (dy/dist)*f*0.02;
    }
    n.vx += (w/2-n.x)*0.0005; n.vy += (h/2-n.y)*0.0005;
  }
  for (var ei=0; ei<graphEdges.length; ei++) {
    var e = graphEdges[ei];
    var a = graphNodes.find(function(n) { return n.id === e.from; });
    var b = graphNodes.find(function(n) { return n.id === e.to; });
    if (!a || !b) continue;
    var dx = b.x-a.x, dy = b.y-a.y, dist = Math.sqrt(dx*dx+dy*dy)+1;
    var idealDist = e.kind === 'hdc' ? 160 : 100;
    var force = (dist - idealDist) * 0.003;
    a.vx += (dx/dist)*force; a.vy += (dy/dist)*force;
    b.vx -= (dx/dist)*force; b.vy -= (dy/dist)*force;
  }
  for (var i=0; i<graphNodes.length; i++) {
    var n = graphNodes[i];
    n.x += n.vx; n.y += n.vy;
    n.x = Math.max(20, Math.min(w-20, n.x)); n.y = Math.max(20, Math.min(h-20, n.y));
    n.pulse = Math.max(0, n.pulse - 0.005);
  }
  // Draw edges
  for (var ei=0; ei<graphEdges.length; ei++) {
    var e = graphEdges[ei];
    var a = graphNodes.find(function(n) { return n.id === e.from; });
    var b = graphNodes.find(function(n) { return n.id === e.to; });
    if (!a || !b) continue;
    var isHdc = e.kind === 'hdc';
    var edgeAlpha = isHdc ? 0.25 : 0.18;
    var edgeColor = isHdc ? '124,111,247' : '34,211,238';
    if (e.kind === 'enabled_by') { edgeColor = '74,222,128'; edgeAlpha = 0.3; }
    graphCtx.strokeStyle = 'rgba(' + edgeColor + ',' + edgeAlpha + ')';
    graphCtx.lineWidth = isHdc ? 1.5 : 1;
    if (isHdc) graphCtx.setLineDash([4,4]); else graphCtx.setLineDash([]);
    graphCtx.beginPath(); graphCtx.moveTo(a.x,a.y); graphCtx.lineTo(b.x,b.y); graphCtx.stroke();
    graphCtx.setLineDash([]);
    // Arrow for enabled_by
    if (e.kind === 'enabled_by') {
      var mx = (a.x + b.x)/2, my = (a.y + b.y)/2;
      var angle = Math.atan2(b.y-a.y, b.x-a.x);
      graphCtx.save(); graphCtx.translate(mx, my); graphCtx.rotate(angle);
      graphCtx.fillStyle = 'rgba(74,222,128,' + edgeAlpha + ')';
      graphCtx.beginPath(); graphCtx.moveTo(5,0); graphCtx.lineTo(-3,-3); graphCtx.lineTo(-3,3); graphCtx.closePath();
      graphCtx.fill(); graphCtx.restore();
    }
    // Similarity label for HDC edges
    if (isHdc && e.similarity) {
      var mx = (a.x+b.x)/2, my = (a.y+b.y)/2;
      graphCtx.fillStyle = 'rgba(165,148,255,0.3)'; graphCtx.font = '8px monospace'; graphCtx.textAlign = 'center';
      graphCtx.fillText(e.similarity.toFixed(2), mx, my - 4);
    }
  }
  // Draw nodes
  for (var i=0; i<graphNodes.length; i++) {
    var n = graphNodes[i];
    var c = I_COLORS[n.kind] || I_COLORS.insight;
    var r = n.radius + n.pulse * 4;
    var isSelected = n.id === state.selectedNode;
    var isHovered = n.id === state.hoveredNode;
    var isHighlighted = hdcHighlights.has(n.id);
    // Glow
    if (n.pulse > 0.1 || isHighlighted) {
      var gr = graphCtx.createRadialGradient(n.x,n.y,0,n.x,n.y,r*3);
      var glowAlpha = isHighlighted ? 0.4 : n.pulse*0.3;
      gr.addColorStop(0,'rgba('+c.r+','+c.g+','+c.b+','+glowAlpha+')');
      gr.addColorStop(1,'rgba('+c.r+','+c.g+','+c.b+',0)');
      graphCtx.fillStyle = gr; graphCtx.beginPath(); graphCtx.arc(n.x,n.y,r*3,0,Math.PI*2); graphCtx.fill();
    }
    // Node
    graphCtx.fillStyle = 'rgba('+c.r+','+c.g+','+c.b+','+(0.5+n.pulse*0.5)+')';
    graphCtx.beginPath(); graphCtx.arc(n.x,n.y,r,0,Math.PI*2); graphCtx.fill();
    // Selection ring
    if (isSelected || isHovered) {
      graphCtx.strokeStyle = isSelected ? '#fff' : 'rgba(255,255,255,0.4)';
      graphCtx.lineWidth = 2; graphCtx.beginPath(); graphCtx.arc(n.x,n.y,r+3,0,Math.PI*2); graphCtx.stroke();
    }
    // HDC highlight ring
    if (isHighlighted) {
      graphCtx.strokeStyle = 'rgba(124,111,247,0.6)';
      graphCtx.lineWidth = 1.5; graphCtx.setLineDash([3,3]);
      graphCtx.beginPath(); graphCtx.arc(n.x,n.y,r+6,0,Math.PI*2); graphCtx.stroke();
      graphCtx.setLineDash([]);
    }
    // Confirm/challenge badges
    if (n.conf > 0) {
      graphCtx.fillStyle = 'rgba(74,222,128,0.8)'; graphCtx.font = '8px monospace'; graphCtx.textAlign = 'center';
      graphCtx.fillText('+'+n.conf, n.x, n.y-r-4);
    }
    if (n.chall > 0) {
      graphCtx.fillStyle = 'rgba(248,113,113,0.8)'; graphCtx.textAlign = 'center';
      graphCtx.fillText('-'+n.chall, n.x, n.y+r+10);
    }
    // Author label
    if (n.author && (isHovered || isSelected)) {
      graphCtx.fillStyle = 'rgba(139,143,168,0.7)'; graphCtx.font = '8px monospace'; graphCtx.textAlign = 'center';
      graphCtx.fillText(n.author.slice(0, 16), n.x, n.y + r + (n.chall > 0 ? 20 : 10));
    }
  }
  // Legend
  graphCtx.font = '9px monospace'; graphCtx.textAlign = 'left';
  var lx = 12, ly = h - 80;
  var kinds = Object.keys(I_COLORS);
  for (var ki=0; ki<kinds.length; ki++) {
    var kc = I_COLORS[kinds[ki]];
    graphCtx.fillStyle = 'rgba('+kc.r+','+kc.g+','+kc.b+',0.7)';
    graphCtx.beginPath(); graphCtx.arc(lx, ly, 4, 0, Math.PI*2); graphCtx.fill();
    graphCtx.fillStyle = 'rgba(139,143,168,0.5)';
    graphCtx.fillText(kinds[ki], lx+10, ly+3);
    ly += 13;
  }
}

export function canvasToNode(ev) {
  for (var i = graphNodes.length-1; i>=0; i--) {
    var n = graphNodes[i];
    var dx = ev.offsetX - n.x, dy = ev.offsetY - n.y;
    if (dx*dx + dy*dy < 15*15) return n;
  }
  return null;
}

export async function openDetail(node) {
  var ins = state.insights.get(node.id);
  renderDetail(node, ins);
  // Query HDC-similar via REST API
  var query = (node.content || '').slice(0, 200);
  if (!query) return;
  try {
    var res = await api('/knowledge/search?q=' + encodeURIComponent(query) + '&k=8');
    var result = res.data;
    if (!result || !result.results) return;
    var similar = result.results.filter(function(h) {
      var rid = (h.id || '').replace(/^insight:/, '');
      return rid && rid !== node.id;
    });
    // Highlight in graph
    hdcHighlights.clear();
    hdcHighlights.add(node.id);
    for (var i = 0; i < similar.length; i++) {
      var rid = (similar[i].id || '').replace(/^insight:/, '');
      hdcHighlights.add(rid);
    }
    // Draw temporary HDC edges (user-interaction, not REST-sourced)
    for (var ei = graphEdges.length-1; ei >= 0; ei--) {
      if (graphEdges[ei].kind === 'hdc' && graphEdges[ei].source !== 'rest') graphEdges.splice(ei,1);
    }
    for (var si = 0; si < Math.min(5, similar.length); si++) {
      var srid = (similar[si].id || '').replace(/^insight:/, '');
      if (graphNodes.find(function(x) { return x.id === srid; })) {
        graphEdges.push({from: node.id, to: srid, kind: 'hdc', source: 'click'});
      }
    }
    // Render side panel similars
    var simContainer = document.getElementById('detail-similar-list');
    if (simContainer) {
      simContainer.innerHTML = '';
      for (var di = 0; di < Math.min(8, similar.length); di++) {
        var h = similar[di];
        var div = document.createElement('div');
        div.className = 'similar-row';
        var drid = (h.id || '').replace(/^insight:/, '');
        (function(capturedRid) {
          div.onclick = function() {
            var target = graphNodes.find(function(x) { return x.id === capturedRid; });
            if (target) { state.selectedNode = capturedRid; openDetail(target); }
          };
        })(drid);
        div.innerHTML =
          '<div class="sim-head"><span>' + h.kind + '</span><span class="sim-score">' + ((h.similarity||0)*100).toFixed(1) + '%</span></div>' +
          '<div class="sim-content">' + (h.content||'').slice(0,120) + '</div>';
        simContainer.appendChild(div);
      }
    }
  } catch (e) { /* ignore */ }
}

export function renderDetail(node, ins) {
  var panel = document.getElementById('detail-panel');
  var meta = document.getElementById('detail-meta');
  if (!node) {
    panel.innerHTML = '<div class="detail-empty">click any node in the knowledge graph to see its content, similar insights (via HDC), and confirm/challenge actions</div>';
    meta.textContent = 'click a node';
    return;
  }
  meta.textContent = node.kind.replace('_', ' ');
  var id = node.id;
  panel.innerHTML =
    '<div class="detail-head">' +
      '<span class="detail-title">' + node.kind.replace('_', ' ') + '</span>' +
      '<span class="detail-close" onclick="state.selectedNode=null; hdcHighlights.clear(); renderDetail(null);">&#10005;</span>' +
    '</div>' +
    '<div class="detail-content">' + (node.content || '(no content)') + '</div>' +
    '<div class="detail-kv">' +
      '<span class="k">id</span><span class="v mono">' + id + '</span>' +
      '<span class="k">author</span><span class="v">' + (ins ? ins.author || 'chain' : 'chain') + '</span>' +
      '<span class="k">confirmations</span><span class="v green">' + node.conf + '</span>' +
      '<span class="k">challenges</span><span class="v red">' + node.chall + '</span>' +
      '<span class="k">weight</span><span class="v">' + (ins ? (ins.weight || 1).toFixed(3) : '1.000') + '</span>' +
      '<span class="k">state</span><span class="v">' + (ins ? ins.state || 'active' : 'active') + '</span>' +
    '</div>' +
    '<div class="detail-actions">' +
      '<button class="btn sm" onclick="doConfirm(\'' + id + '\')">CONFIRM</button>' +
      '<button class="btn ghost sm" onclick="doChallenge(\'' + id + '\')">CHALLENGE</button>' +
    '</div>' +
    '<div class="detail-similar-title">HDC-similar insights</div>' +
    '<div id="detail-similar-list"></div>';
}

export async function doConfirm(id) {
  try {
    await apiPost('/knowledge/entries/' + id + '/confirm', {confirmer: 'human-operator'});
    toast('ok', 'confirmed');
    var n = graphNodes.find(function(x) { return x.id === id; }); if (n) { n.conf++; n.pulse = 1; }
    state.confirmsCount++;
  } catch(e) { toast('err', 'confirm failed: ' + e.message); }
}

export async function doChallenge(id) {
  try {
    await apiPost('/knowledge/entries/' + id + '/challenge', {challenger: 'human-operator'});
    toast('warn', 'challenged');
    var n = graphNodes.find(function(x) { return x.id === id; }); if (n) { n.chall++; n.pulse = 1; }
    state.challengesCount++;
  } catch(e) { toast('err', 'challenge failed: ' + e.message); }
}

/* Reset dimensions on resize (called from main.js) */
export function resetGraphSize() { graphW = 0; graphH = 0; }

/* Expose globally for inline onclick handlers in renderDetail */
window.doConfirm = doConfirm;
window.doChallenge = doChallenge;
window.renderDetail = renderDetail;
window.state = state;
window.hdcHighlights = hdcHighlights;
window.graphNodes = graphNodes;
