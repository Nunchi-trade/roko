/* ================================================================
   PHEROMONE PARTICLE SYSTEM — canvas rendering + particle deposit
   ================================================================ */

import { state } from './state.js';

export var P_COLORS = {
  threat: {r:248,g:113,b:113}, opportunity: {r:74,g:222,b:128}, wisdom: {r:251,g:191,b:36},
};
export var P_HALFLIFE = { threat: 60, opportunity: 90, wisdom: 180 };
var pheroW=0, pheroH=0;

export function resizeCanvas(c) {
  var rect = c.getBoundingClientRect();
  var dpr = window.devicePixelRatio || 1;
  c.width = rect.width * dpr; c.height = rect.height * dpr;
  var ctx = c.getContext('2d'); ctx.scale(dpr, dpr);
  return {w: rect.width, h: rect.height};
}

export function depositPheromoneParticle(kind, content, intensity, chainId) {
  if (chainId === undefined) chainId = null;
  var pheroCanvas = document.getElementById('phero-canvas');
  var w = pheroCanvas.clientWidth, h = pheroCanvas.clientHeight;
  var regions = {
    threat: {cx: w * 0.22, cy: h * 0.30, r: Math.min(w, h) * 0.15},
    opportunity: {cx: w * 0.78, cy: h * 0.30, r: Math.min(w, h) * 0.15},
    wisdom: {cx: w * 0.50, cy: h * 0.75, r: Math.min(w, h) * 0.15},
  };
  var reg = regions[kind] || regions.wisdom;
  var angle = Math.random() * 2 * Math.PI;
  var rad = Math.random() * reg.r;
  state.pheromones.push({
    kind: kind, content: content, intensity: Math.max(0.1, Math.min(1, intensity)),
    x: reg.cx + Math.cos(angle) * rad, y: reg.cy + Math.sin(angle) * rad,
    anchorX: reg.cx, anchorY: reg.cy,
    vx: (Math.random() - 0.5) * 0.6, vy: (Math.random() - 0.5) * 0.6,
    age: 0, deposited: Date.now(), halfLife: P_HALFLIFE[kind] || 90,
    pulse: 1, chainId: chainId, decayProjection: null,
  });
  while (state.pheromones.length > 500) state.pheromones.shift();
}

export function drawPheromones(dt) {
  var pheroCanvas = document.getElementById('phero-canvas');
  var pheroCtx = pheroCanvas.getContext('2d');
  var w = pheroCanvas.clientWidth, h = pheroCanvas.clientHeight;
  if (pheroW !== w || pheroH !== h) { var d = resizeCanvas(pheroCanvas); pheroW = d.w; pheroH = d.h; }
  pheroCtx.clearRect(0, 0, w, h);
  // Grid background
  pheroCtx.strokeStyle = 'rgba(79,83,112,0.08)';
  pheroCtx.lineWidth = 0.5;
  for (var gx = 0; gx < w; gx += 40) { pheroCtx.beginPath(); pheroCtx.moveTo(gx,0); pheroCtx.lineTo(gx,h); pheroCtx.stroke(); }
  for (var gy = 0; gy < h; gy += 40) { pheroCtx.beginPath(); pheroCtx.moveTo(0,gy); pheroCtx.lineTo(w,gy); pheroCtx.stroke(); }
  // Region labels
  var labelAlpha = 0.18;
  pheroCtx.font = '11px monospace'; pheroCtx.textAlign = 'center';
  pheroCtx.fillStyle = 'rgba(248,113,113,' + labelAlpha + ')'; pheroCtx.fillText('THREAT', w*0.22, h*0.15);
  pheroCtx.fillStyle = 'rgba(74,222,128,' + labelAlpha + ')'; pheroCtx.fillText('OPPORTUNITY', w*0.78, h*0.15);
  pheroCtx.fillStyle = 'rgba(251,191,36,' + labelAlpha + ')'; pheroCtx.fillText('WISDOM', w*0.50, h*0.60);
  // Region arcs
  pheroCtx.setLineDash([4,6]);
  var regions = [
    {cx: w*0.22, cy: h*0.30, r: Math.min(w,h)*0.15, c:'rgba(248,113,113,0.08)'},
    {cx: w*0.78, cy: h*0.30, r: Math.min(w,h)*0.15, c:'rgba(74,222,128,0.08)'},
    {cx: w*0.50, cy: h*0.75, r: Math.min(w,h)*0.15, c:'rgba(251,191,36,0.08)'},
  ];
  for (var ri = 0; ri < regions.length; ri++) {
    pheroCtx.strokeStyle = regions[ri].c; pheroCtx.beginPath();
    pheroCtx.arc(regions[ri].cx, regions[ri].cy, regions[ri].r, 0, Math.PI*2);
    pheroCtx.stroke();
  }
  pheroCtx.setLineDash([]);
  // Particles
  var now = Date.now();
  var frames = 0, lastFpsT = performance.now(), pheroFps = 0;
  var alive = [];
  var counts = {threat:0, opportunity:0, wisdom:0};
  for (var i = 0; i < state.pheromones.length; i++) {
    var p = state.pheromones[i];
    var elapsed = (now - p.deposited) / 1000;
    p.pulse = p.intensity * Math.pow(0.5, elapsed / p.halfLife);
    if (p.pulse < 0.02) continue;
    alive.push(p);
    counts[p.kind] = (counts[p.kind] || 0) + 1;
    // Drift toward anchor
    p.vx += (p.anchorX - p.x) * 0.0003;
    p.vy += (p.anchorY - p.y) * 0.0003;
    p.vx *= 0.995; p.vy *= 0.995;
    p.x += p.vx; p.y += p.vy;
    // Clamp
    p.x = Math.max(5, Math.min(w-5, p.x));
    p.y = Math.max(5, Math.min(h-5, p.y));
    var c = P_COLORS[p.kind] || P_COLORS.wisdom;
    var r = 3 + p.pulse * 8;
    // Outer glow
    var grad = pheroCtx.createRadialGradient(p.x, p.y, 0, p.x, p.y, r * 3);
    grad.addColorStop(0, 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',' + (p.pulse * 0.3) + ')');
    grad.addColorStop(1, 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',0)');
    pheroCtx.fillStyle = grad;
    pheroCtx.beginPath(); pheroCtx.arc(p.x, p.y, r * 3, 0, Math.PI * 2); pheroCtx.fill();
    // Core
    pheroCtx.fillStyle = 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',' + p.pulse + ')';
    pheroCtx.beginPath(); pheroCtx.arc(p.x, p.y, r, 0, Math.PI * 2); pheroCtx.fill();
    // Content label (if pulse > 0.4)
    if (p.pulse > 0.4 && p.content) {
      pheroCtx.fillStyle = 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',' + (p.pulse * 0.6) + ')';
      pheroCtx.font = '9px monospace'; pheroCtx.textAlign = 'center';
      pheroCtx.fillText(p.content.slice(0, 28), p.x, p.y - r - 3);
    }
    // Decay projection visualization
    if (p.decayProjection && p.pulse > 0.15) {
      var dp = p.decayProjection;
      // Small arc showing decay forecast: 3 ticks at 1h, 4h, 24h
      var arcR = r * 2.5;
      var projections = [
        { t: '1h', val: dp.in_1h || 0, angle: -Math.PI * 0.6 },
        { t: '4h', val: dp.in_4h || 0, angle: -Math.PI * 0.3 },
        { t: '24h', val: dp.in_24h || 0, angle: 0 },
      ];
      for (var dpi = 0; dpi < projections.length; dpi++) {
        var proj = projections[dpi];
        var projAlpha = Math.min(0.6, (proj.val / p.intensity) * 0.5);
        if (projAlpha < 0.02) continue;
        var px = p.x + Math.cos(proj.angle) * arcR;
        var py = p.y + Math.sin(proj.angle) * arcR;
        // Tiny dot
        pheroCtx.fillStyle = 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',' + projAlpha + ')';
        pheroCtx.beginPath(); pheroCtx.arc(px, py, 1.5, 0, Math.PI * 2); pheroCtx.fill();
        // Connecting line
        pheroCtx.strokeStyle = 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',' + (projAlpha * 0.4) + ')';
        pheroCtx.lineWidth = 0.5;
        pheroCtx.beginPath(); pheroCtx.moveTo(p.x + Math.cos(proj.angle) * r, p.y + Math.sin(proj.angle) * r);
        pheroCtx.lineTo(px, py); pheroCtx.stroke();
        // Label
        pheroCtx.fillStyle = 'rgba(' + c.r + ',' + c.g + ',' + c.b + ',' + (projAlpha * 0.8) + ')';
        pheroCtx.font = '7px monospace'; pheroCtx.textAlign = 'center';
        pheroCtx.fillText(proj.t + ':' + proj.val.toFixed(2), px, py - 4);
      }
    }
  }
  // Inter-particle connections
  for (var i = 0; i < alive.length; i++) {
    var a = alive[i];
    for (var j = i + 1; j < alive.length && j < i + 20; j++) {
      var b = alive[j];
      var cdx = a.x-b.x, cdy = a.y-b.y, cd = Math.sqrt(cdx*cdx+cdy*cdy);
      if (cd < 80) {
        var cc = P_COLORS[a.kind];
        var alpha = (1-cd/80) * Math.min(a.pulse, b.pulse) * 0.2;
        pheroCtx.strokeStyle = 'rgba(' + cc.r + ',' + cc.g + ',' + cc.b + ',' + alpha + ')';
        pheroCtx.lineWidth = 1;
        pheroCtx.beginPath(); pheroCtx.moveTo(a.x,a.y); pheroCtx.lineTo(b.x,b.y); pheroCtx.stroke();
      }
    }
  }
  frames++;
  var tN = performance.now();
  if (tN - lastFpsT > 500) { pheroFps = Math.round(frames*1000/(tN-lastFpsT)); frames=0; lastFpsT = tN; }
  document.getElementById('phero-particles').textContent = state.pheromones.length;
  document.getElementById('phero-fps').textContent = pheroFps + ' fps';
  document.getElementById('phero-c-threat').textContent = counts.threat;
  document.getElementById('phero-c-opp').textContent = counts.opportunity;
  document.getElementById('phero-c-wisdom').textContent = counts.wisdom;
}

/* Reset dimensions on resize (called from main.js) */
export function resetPheroSize() { pheroW = 0; pheroH = 0; }
