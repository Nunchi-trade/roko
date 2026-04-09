/* ================================================================
   PHEROMONE PARTICLE SYSTEM — force-directed bubble visualization
   ================================================================
   Force sim, shaped particles (diamond/circle/hexagon), spatial grid,
   hover/click interactions, entrance/death animations, decay ghosts.
   ================================================================ */

import { state } from './state.js';

// ── Exports (keep API compatible with polling.js / ws.js / main.js) ──

export var P_COLORS = {
  threat:      { r: 248, g: 113, b: 113 },
  opportunity: { r: 74,  g: 222, b: 128 },
  wisdom:      { r: 251, g: 191, b: 36  },
};
export var P_HALFLIFE = { threat: 60, opportunity: 90, wisdom: 180 };

// ── Internal state ──

var pheroW = 0, pheroH = 0;
var bgCanvas = null;           // offscreen cached background
var bgDirty = true;
var hoveredParticle = null;
var selectedParticle = null;    // pinned tooltip
var kindFilters = { threat: true, opportunity: true, wisdom: true };
var fpsFrames = 0, fpsLast = performance.now(), fpsCurrent = 60;

// ── Spatial grid ──

var GRID_CELL = 60;
var gridCols = 0, gridRows = 0;
var grid = [];  // flat array of arrays

function gridBuild(particles, w, h) {
  gridCols = Math.ceil(w / GRID_CELL) || 1;
  gridRows = Math.ceil(h / GRID_CELL) || 1;
  var len = gridCols * gridRows;
  if (grid.length !== len) {
    grid = new Array(len);
    for (var i = 0; i < len; i++) grid[i] = [];
  } else {
    for (var i = 0; i < len; i++) grid[i].length = 0;
  }
  for (var i = 0; i < particles.length; i++) {
    var p = particles[i];
    var col = Math.min(gridCols - 1, Math.max(0, (p.x / GRID_CELL) | 0));
    var row = Math.min(gridRows - 1, Math.max(0, (p.y / GRID_CELL) | 0));
    grid[row * gridCols + col].push(p);
  }
}

function gridNeighbors(p, radius) {
  var results = [];
  var col = (p.x / GRID_CELL) | 0;
  var row = (p.y / GRID_CELL) | 0;
  var cr = Math.ceil(radius / GRID_CELL);
  for (var dr = -cr; dr <= cr; dr++) {
    var r = row + dr;
    if (r < 0 || r >= gridRows) continue;
    for (var dc = -cr; dc <= cr; dc++) {
      var c = col + dc;
      if (c < 0 || c >= gridCols) continue;
      var cell = grid[r * gridCols + c];
      for (var i = 0; i < cell.length; i++) {
        if (cell[i] !== p) results.push(cell[i]);
      }
    }
  }
  return results;
}

function gridHitTest(mx, my, particles) {
  var col = (mx / GRID_CELL) | 0;
  var row = (my / GRID_CELL) | 0;
  var best = null, bestD2 = Infinity;
  for (var dr = -1; dr <= 1; dr++) {
    var r = row + dr;
    if (r < 0 || r >= gridRows) continue;
    for (var dc = -1; dc <= 1; dc++) {
      var c = col + dc;
      if (c < 0 || c >= gridCols) continue;
      var cell = grid[r * gridCols + c];
      for (var i = 0; i < cell.length; i++) {
        var p = cell[i];
        var dx = mx - p.x, dy = my - p.y;
        var d2 = dx * dx + dy * dy;
        var hr = p._drawR || 10;
        if (d2 < hr * hr * 2.5 && d2 < bestD2) { best = p; bestD2 = d2; }
      }
    }
  }
  return best;
}

// ── Kind centroids ──

var centroids = { threat: { x: 0, y: 0, n: 0 }, opportunity: { x: 0, y: 0, n: 0 }, wisdom: { x: 0, y: 0, n: 0 } };

function updateCentroids(alive, w, h) {
  // Reset
  var cx = w / 2, cy = h / 2;
  centroids.threat.x = cx * 0.4;  centroids.threat.y = cy * 0.6;  centroids.threat.n = 0;
  centroids.opportunity.x = cx * 1.6; centroids.opportunity.y = cy * 0.6; centroids.opportunity.n = 0;
  centroids.wisdom.x = cx; centroids.wisdom.y = cy * 1.4; centroids.wisdom.n = 0;

  // Accumulate
  var sums = { threat: { x: 0, y: 0, n: 0 }, opportunity: { x: 0, y: 0, n: 0 }, wisdom: { x: 0, y: 0, n: 0 } };
  for (var i = 0; i < alive.length; i++) {
    var p = alive[i];
    var s = sums[p.kind];
    if (!s) continue;
    s.x += p.x; s.y += p.y; s.n++;
  }
  for (var k in sums) {
    if (sums[k].n > 2) {
      centroids[k].x = sums[k].x / sums[k].n;
      centroids[k].y = sums[k].y / sums[k].n;
    }
    centroids[k].n = sums[k].n;
  }
}

// ── Force simulation ──

function applyForces(alive, w, h) {
  var cx = w / 2, cy = h / 2;

  for (var i = 0; i < alive.length; i++) {
    var p = alive[i];

    // Kind attraction toward centroid (gentle drift)
    var cent = centroids[p.kind];
    if (cent) {
      p.vx += (cent.x - p.x) * 0.0003;
      p.vy += (cent.y - p.y) * 0.0003;
    }

    // Weak centering
    p.vx += (cx - p.x) * 0.00015;
    p.vy += (cy - p.y) * 0.00015;

    // Repulsion via spatial grid (softer)
    var neighbors = gridNeighbors(p, 60);
    for (var j = 0; j < neighbors.length; j++) {
      var q = neighbors[j];
      var dx = p.x - q.x, dy = p.y - q.y;
      var d2 = dx * dx + dy * dy;
      if (d2 < 1) d2 = 1;
      if (d2 < 3600) { // 60²
        var f = 12 / d2; // softer repulsion
        p.vx += dx * f;
        p.vy += dy * f;
      }
    }

    // Heavy damping — slow, ambient drift
    p.vx *= 0.78;
    p.vy *= 0.78;

    // Integrate
    p.x += p.vx;
    p.y += p.vy;

    // Clamp to canvas with margin
    p.x = Math.max(12, Math.min(w - 12, p.x));
    p.y = Math.max(12, Math.min(h - 12, p.y));
  }
}

// ── Shape renderers ──

function drawDiamond(ctx, x, y, r) {
  ctx.beginPath();
  ctx.moveTo(x, y - r);
  ctx.lineTo(x + r, y);
  ctx.lineTo(x, y + r);
  ctx.lineTo(x - r, y);
  ctx.closePath();
}

function drawHexagon(ctx, x, y, r) {
  ctx.beginPath();
  for (var i = 0; i < 6; i++) {
    var angle = Math.PI / 6 + i * Math.PI / 3;
    var px = x + r * Math.cos(angle);
    var py = y + r * Math.sin(angle);
    if (i === 0) ctx.moveTo(px, py);
    else ctx.lineTo(px, py);
  }
  ctx.closePath();
}

function drawShape(ctx, kind, x, y, r) {
  if (kind === 'threat') drawDiamond(ctx, x, y, r);
  else if (kind === 'wisdom') drawHexagon(ctx, x, y, r);
  else { ctx.beginPath(); ctx.arc(x, y, r, 0, Math.PI * 2); }
}

// ── Background (offscreen cached) ──

function ensureBgCanvas(w, h) {
  if (!bgCanvas || bgCanvas.width !== w || bgCanvas.height !== h || bgDirty) {
    if (!bgCanvas) bgCanvas = document.createElement('canvas');
    bgCanvas.width = w; bgCanvas.height = h;
    var bctx = bgCanvas.getContext('2d');
    bctx.clearRect(0, 0, w, h);

    // Subtle dot grid
    bctx.fillStyle = 'rgba(79,83,112,0.04)';
    for (var gx = 0; gx < w; gx += 30) {
      for (var gy = 0; gy < h; gy += 30) {
        bctx.beginPath();
        bctx.arc(gx, gy, 0.7, 0, Math.PI * 2);
        bctx.fill();
      }
    }
    bgDirty = false;
  }
}

function drawGravityWells(ctx, w, h) {
  for (var k in centroids) {
    var c = centroids[k];
    if (c.n < 1) continue;
    var col = P_COLORS[k];
    var grad = ctx.createRadialGradient(c.x, c.y, 0, c.x, c.y, Math.min(w, h) * 0.18);
    grad.addColorStop(0, 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',0.03)');
    grad.addColorStop(1, 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',0)');
    ctx.fillStyle = grad;
    ctx.fillRect(0, 0, w, h);
  }
}

function drawCentroidLabels(ctx, t) {
  var breathAlpha = 0.15 + 0.05 * Math.sin(t / 1200);
  ctx.font = '10px monospace';
  ctx.textAlign = 'center';
  for (var k in centroids) {
    var c = centroids[k];
    if (c.n < 1) continue;
    var col = P_COLORS[k];
    ctx.fillStyle = 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',' + breathAlpha + ')';
    ctx.fillText(k.toUpperCase() + ' (' + c.n + ')', c.x, c.y - Math.min(40, 20 + c.n * 0.3));
  }
}

// ── Entrance / death animation helpers ──

function spawnParticle(kind, content, intensity, chainId, w, h) {
  var cent = centroids[kind] || { x: w / 2, y: h / 2 };
  // Enter at perimeter of cluster
  var angle = Math.random() * Math.PI * 2;
  var dist = 40 + Math.random() * 30;
  return {
    kind: kind,
    content: content,
    intensity: Math.max(0.1, Math.min(1, intensity)),
    x: cent.x + Math.cos(angle) * dist,
    y: cent.y + Math.sin(angle) * dist,
    vx: -Math.cos(angle) * 0.1,  // gentle drift inward
    vy: -Math.sin(angle) * 0.1,
    deposited: Date.now(),
    halfLife: P_HALFLIFE[kind] || 90,
    pulse: 1,
    chainId: chainId,
    confirmations: 0,
    decayProjection: null,
    // Animation state
    _enterT: performance.now(),   // entrance start time
    _dying: false,
    _deathT: 0,
    _drawR: 0,
  };
}

// ── Tooltip helpers (exported for main.js) ──

export function handlePheroMouseMove(ev) {
  if (selectedParticle) return; // pinned
  var canvas = document.getElementById('phero-canvas');
  var rect = canvas.getBoundingClientRect();
  var mx = ev.clientX - rect.left;
  var my = ev.clientY - rect.top;
  var hit = gridHitTest(mx, my, state.pheromones);
  if (hit && !kindFilters[hit.kind]) hit = null;
  hoveredParticle = hit;
  showTooltip(hit, ev.offsetX, ev.offsetY);
  canvas.style.cursor = hit ? 'pointer' : 'default';
}

export function handlePheroClick(ev) {
  var canvas = document.getElementById('phero-canvas');
  var rect = canvas.getBoundingClientRect();
  var mx = ev.clientX - rect.left;
  var my = ev.clientY - rect.top;
  var hit = gridHitTest(mx, my, state.pheromones);
  if (hit && !kindFilters[hit.kind]) hit = null;

  if (hit) {
    selectedParticle = hit;
    hoveredParticle = hit;
    showTooltip(hit, ev.offsetX, ev.offsetY, true);
    // Fetch decay projection if available
    if (hit.chainId) {
      fetchDecayProjection(hit);
    }
  } else {
    selectedParticle = null;
    hoveredParticle = null;
    hideTooltip();
  }
}

export function handlePheroMouseLeave() {
  if (!selectedParticle) {
    hoveredParticle = null;
    hideTooltip();
  }
}

export function handlePheroFilterClick(ev) {
  var btn = ev.target.closest('.phero-filter-pill');
  if (!btn) return;
  var kind = btn.dataset.kind;
  kindFilters[kind] = !kindFilters[kind];
  btn.classList.toggle('active', kindFilters[kind]);
}

function showTooltip(p, ox, oy, pinned) {
  var tt = document.getElementById('phero-tooltip');
  if (!p) { hideTooltip(); return; }

  var now = Date.now();
  var elapsed = (now - p.deposited) / 1000;
  var decayFactor = Math.pow(0.5, elapsed / p.halfLife);
  var currentIntensity = p.intensity * decayFactor;
  var ago = elapsed < 60 ? Math.round(elapsed) + 's' : Math.round(elapsed / 60) + 'm';

  // Decay projections
  var proj1h  = p.intensity * Math.pow(0.5, (elapsed + 3600) / p.halfLife);
  var proj4h  = p.intensity * Math.pow(0.5, (elapsed + 14400) / p.halfLife);
  var proj24h = p.intensity * Math.pow(0.5, (elapsed + 86400) / p.halfLife);

  var barPct = Math.round(currentIntensity * 100);

  var html =
    '<div class="pt-kind ' + p.kind + '">' + p.kind + '</div>' +
    '<div class="pt-content">' + (p.content || '(no content)') + '</div>' +
    '<div class="pt-bar-row">' +
      '<span class="pt-bar-label">intensity</span>' +
      '<span class="pt-bar-track"><span class="pt-bar-fill ' + p.kind + '" style="width:' + barPct + '%"></span></span>' +
      '<span class="pt-bar-val">' + currentIntensity.toFixed(2) + '</span>' +
    '</div>' +
    '<div class="pt-meta">' +
      '<span>deposited <b>' + ago + ' ago</b></span>' +
      '<span>half-life <b>' + p.halfLife + 's</b></span>' +
      (p.confirmations > 0 ? '<span>confirms <b>' + p.confirmations + '</b></span>' : '') +
    '</div>' +
    '<div class="pt-decay">' +
      '<span>1h: <b>' + proj1h.toFixed(3) + '</b></span>' +
      '<span>4h: <b>' + proj4h.toFixed(3) + '</b></span>' +
      '<span>24h: <b>' + proj24h.toFixed(3) + '</b></span>' +
    '</div>';

  // Decay sparkline from fetched projection
  if (p.decayProjection && pinned) {
    html += '<canvas class="pt-sparkline" id="phero-sparkline-canvas" width="200" height="24"></canvas>';
  }

  tt.innerHTML = html;
  tt.style.display = 'block';
  tt.classList.toggle('pinned', !!pinned);

  // Position (keep on screen)
  var wrap = document.querySelector('.phero-canvas-wrap');
  var ww = wrap.clientWidth, wh = wrap.clientHeight;
  var tx = ox + 16, ty = oy + 16;
  if (tx + 360 > ww) tx = ox - 370;
  if (ty + 200 > wh) ty = Math.max(10, wh - 210);
  tt.style.left = tx + 'px';
  tt.style.top = ty + 'px';

  // Draw sparkline if projection data exists
  if (p.decayProjection && pinned) {
    requestAnimationFrame(function() { drawDecaySparkline(p); });
  }
}

function hideTooltip() {
  var tt = document.getElementById('phero-tooltip');
  if (tt) { tt.style.display = 'none'; tt.classList.remove('pinned'); }
}

async function fetchDecayProjection(p) {
  if (!p.chainId) return;
  try {
    var res = await fetch('/api/pheromones/' + encodeURIComponent(p.chainId) + '/projection');
    if (res.ok) {
      p.decayProjection = await res.json();
      if (selectedParticle === p) showTooltip(p, 0, 0, true);
    }
  } catch (_) {}
}

function drawDecaySparkline(p) {
  var canvas = document.getElementById('phero-sparkline-canvas');
  if (!canvas || !p.decayProjection) return;
  var ctx = canvas.getContext('2d');
  var dp = p.decayProjection;
  var points = dp.points || [];
  if (points.length < 2) return;
  var w = canvas.width, h = canvas.height;
  ctx.clearRect(0, 0, w, h);

  var maxV = 0;
  for (var i = 0; i < points.length; i++) if (points[i].value > maxV) maxV = points[i].value;
  if (maxV === 0) maxV = 1;

  var col = P_COLORS[p.kind] || P_COLORS.wisdom;
  ctx.strokeStyle = 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',0.7)';
  ctx.lineWidth = 1.5;
  ctx.beginPath();
  for (var i = 0; i < points.length; i++) {
    var px = (i / (points.length - 1)) * w;
    var py = h - (points[i].value / maxV) * (h - 2);
    if (i === 0) ctx.moveTo(px, py); else ctx.lineTo(px, py);
  }
  ctx.stroke();
}

// ── Public API ──

export function resizeCanvas(c) {
  var dpr = window.devicePixelRatio || 1;
  var cw = c.clientWidth, ch = c.clientHeight;
  var targetW = Math.round(cw * dpr), targetH = Math.round(ch * dpr);
  if (c.width !== targetW || c.height !== targetH) {
    c.width = targetW; c.height = targetH;
  }
  var ctx = c.getContext('2d');
  ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
  return { w: cw, h: ch };
}

export function depositPheromoneParticle(kind, content, intensity, chainId) {
  if (chainId === undefined) chainId = null;
  var pheroCanvas = document.getElementById('phero-canvas');
  var w = pheroCanvas.clientWidth, h = pheroCanvas.clientHeight;
  var p = spawnParticle(kind, content, intensity, chainId, w, h);
  state.pheromones.push(p);
  while (state.pheromones.length > 500) state.pheromones.shift();
}

export function drawPheromones(dt) {
  var pheroCanvas = document.getElementById('phero-canvas');
  var ctx = pheroCanvas.getContext('2d');
  var w = pheroCanvas.clientWidth, h = pheroCanvas.clientHeight;
  if (pheroW !== w || pheroH !== h) {
    var d = resizeCanvas(pheroCanvas);
    pheroW = d.w; pheroH = d.h;
    bgDirty = true;
  }
  ctx.clearRect(0, 0, w, h);

  // ── Background ──
  ensureBgCanvas(w, h);
  ctx.drawImage(bgCanvas, 0, 0, w, h);

  // ── Update particles ──
  var now = Date.now();
  var t = performance.now();
  var alive = [];
  var counts = { threat: 0, opportunity: 0, wisdom: 0 };
  var intensitySums = { threat: 0, opportunity: 0, wisdom: 0 };
  var totalIntensity = 0;

  for (var i = state.pheromones.length - 1; i >= 0; i--) {
    var p = state.pheromones[i];
    var elapsed = (now - p.deposited) / 1000;
    p.pulse = p.intensity * Math.pow(0.5, elapsed / p.halfLife);

    // Death animation
    if (p.pulse < 0.02 && !p._dying) {
      p._dying = true;
      p._deathT = t;
    }
    if (p._dying) {
      var deathElapsed = t - p._deathT;
      if (deathElapsed > 300) {
        state.pheromones.splice(i, 1);
        if (hoveredParticle === p) { hoveredParticle = null; hideTooltip(); }
        if (selectedParticle === p) { selectedParticle = null; hideTooltip(); }
        continue;
      }
    }

    if (!kindFilters[p.kind]) continue;
    alive.push(p);
    counts[p.kind] = (counts[p.kind] || 0) + 1;
    intensitySums[p.kind] = (intensitySums[p.kind] || 0) + p.pulse;
    totalIntensity += p.pulse;
  }

  // ── Forces ──
  updateCentroids(alive, w, h);
  gridBuild(alive, w, h);
  applyForces(alive, w, h);

  // ── Gravity wells ──
  drawGravityWells(ctx, w, h);

  // ── Same-kind connections ──
  ctx.lineWidth = 0.5;
  for (var i = 0; i < alive.length; i++) {
    var a = alive[i];
    if (hoveredParticle && a !== hoveredParticle && hoveredParticle.kind !== a.kind) continue;
    var neighbors = gridNeighbors(a, 100);
    var connCount = 0;
    for (var j = 0; j < neighbors.length && connCount < 5; j++) {
      var b = neighbors[j];
      if (b.kind !== a.kind) continue;
      var dx = a.x - b.x, dy = a.y - b.y;
      var d = Math.sqrt(dx * dx + dy * dy);
      if (d > 100) continue;
      var col = P_COLORS[a.kind];
      var alpha = (1 - d / 100) * Math.min(a.pulse, b.pulse) * 0.15;
      if (hoveredParticle && (a === hoveredParticle || b === hoveredParticle)) alpha *= 3;
      ctx.strokeStyle = 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',' + alpha + ')';
      ctx.beginPath(); ctx.moveTo(a.x, a.y); ctx.lineTo(b.x, b.y); ctx.stroke();
      connCount++;
    }
  }

  // ── Draw particles ──
  var isHovering = !!hoveredParticle;

  for (var i = 0; i < alive.length; i++) {
    var p = alive[i];
    var col = P_COLORS[p.kind] || P_COLORS.wisdom;

    // Size
    var baseR = 4 + p.intensity * 14;
    var r = baseR * (0.3 + p.pulse / p.intensity * 0.7);

    // Entrance animation (600ms scale-in)
    var enterElapsed = t - (p._enterT || 0);
    var enterScale = 1;
    var enterFlash = 0;
    if (enterElapsed < 600) {
      enterScale = easeOutCubic(enterElapsed / 600);
      enterFlash = Math.max(0, 1 - enterElapsed / 200); // white flash for first 200ms
    }

    // Death animation
    var deathScale = 1, deathAlpha = 1;
    if (p._dying) {
      var de = (t - p._deathT) / 300;
      deathScale = 1 - de;
      deathAlpha = 1 - de;
    }

    var finalR = r * enterScale * deathScale;
    var finalBaseR = baseR * enterScale * deathScale;
    p._drawR = finalR;

    if (finalR < 0.5) continue;

    // Dim non-hovered particles
    var dimFactor = 1;
    if (isHovering && p !== hoveredParticle) dimFactor = 0.3;

    // ── Ghost outline (original size) ──
    ctx.globalAlpha = 0.08 * dimFactor * deathAlpha;
    drawShape(ctx, p.kind, p.x, p.y, finalBaseR);
    ctx.strokeStyle = 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',1)';
    ctx.lineWidth = 1;
    ctx.stroke();

    // ── Glow (only for pulse > 0.15) ──
    if (p.pulse > 0.15) {
      ctx.globalAlpha = dimFactor * deathAlpha;
      var glowR = finalR * 2.5;
      var grad = ctx.createRadialGradient(p.x, p.y, 0, p.x, p.y, glowR);
      grad.addColorStop(0, 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',' + (p.pulse * 0.2) + ')');
      grad.addColorStop(1, 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',0)');
      ctx.fillStyle = grad;
      ctx.beginPath(); ctx.arc(p.x, p.y, glowR, 0, Math.PI * 2); ctx.fill();
    }

    // ── Core shape ──
    ctx.globalAlpha = (p.pulse * 0.8 + 0.2) * dimFactor * deathAlpha;
    drawShape(ctx, p.kind, p.x, p.y, finalR);
    ctx.fillStyle = 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',' + p.pulse + ')';
    ctx.fill();

    // ── Entrance flash ──
    if (enterFlash > 0) {
      ctx.globalAlpha = enterFlash * 0.6 * dimFactor;
      drawShape(ctx, p.kind, p.x, p.y, finalR);
      ctx.fillStyle = 'rgba(255,255,255,1)';
      ctx.fill();
    }

    // ── Entrance ripple ring ──
    if (enterElapsed < 600) {
      var rippleR = finalR + (enterElapsed / 600) * 30;
      var rippleAlpha = (1 - enterElapsed / 600) * 0.3 * dimFactor;
      ctx.globalAlpha = rippleAlpha;
      ctx.strokeStyle = 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',1)';
      ctx.lineWidth = 1;
      ctx.beginPath(); ctx.arc(p.x, p.y, rippleR, 0, Math.PI * 2); ctx.stroke();
    }

    // ── Confirmation rings ──
    var confs = p.confirmations || 0;
    if (confs > 0) {
      var rings = Math.min(3, confs);
      ctx.globalAlpha = 0.3 * dimFactor * deathAlpha;
      ctx.strokeStyle = 'rgba(' + col.r + ',' + col.g + ',' + col.b + ',0.5)';
      ctx.lineWidth = 0.7;
      for (var ri = 1; ri <= rings; ri++) {
        ctx.beginPath();
        ctx.arc(p.x, p.y, finalR + ri * 4, 0, Math.PI * 2);
        ctx.stroke();
      }
    }

    // ── Selection ring (hovered or pinned) ──
    if (p === hoveredParticle) {
      ctx.globalAlpha = 0.8;
      ctx.strokeStyle = 'rgba(255,255,255,0.9)';
      ctx.lineWidth = 1.5;
      ctx.beginPath();
      ctx.arc(p.x, p.y, finalR + 3, 0, Math.PI * 2);
      ctx.stroke();
    }

    ctx.globalAlpha = 1;
  }

  // ── Centroid labels ──
  drawCentroidLabels(ctx, t);

  // ── FPS ──
  fpsFrames++;
  if (t - fpsLast > 500) {
    fpsCurrent = Math.round(fpsFrames * 1000 / (t - fpsLast));
    fpsFrames = 0;
    fpsLast = t;
  }

  // ── HUD updates ──
  setText('phero-particles', alive.length);
  setText('phero-fps', fpsCurrent);
  setText('phero-c-threat', counts.threat);
  setText('phero-c-opp', counts.opportunity);
  setText('phero-c-wisdom', counts.wisdom);
  setText('phero-intensity', totalIntensity.toFixed(1));

  // Mini bars — height proportional to kind intensity
  var maxKind = Math.max(1, intensitySums.threat, intensitySums.opportunity, intensitySums.wisdom);
  setBarHeight('phero-minibar-threat', intensitySums.threat / maxKind);
  setBarHeight('phero-minibar-opp', intensitySums.opportunity / maxKind);
  setBarHeight('phero-minibar-wisdom', intensitySums.wisdom / maxKind);
}

export function resetPheroSize() { pheroW = 0; pheroH = 0; bgDirty = true; }

// ── Utilities ──

function setText(id, val) {
  var el = document.getElementById(id);
  if (el) el.textContent = val;
}

function setBarHeight(id, frac) {
  var el = document.getElementById(id);
  if (el) el.style.height = Math.max(2, Math.round(frac * 12)) + 'px';
}

function easeOutCubic(t) { return 1 - Math.pow(1 - t, 3); }
