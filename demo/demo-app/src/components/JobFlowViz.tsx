import { useRef, useEffect, useCallback } from 'react';
import './JobFlowViz.css';

/* ── Types ── */

export interface JobFlowJobState {
  id: number;
  state: 'waiting' | 'funded' | 'assigned' | 'submitted' | 'resolved';
  bounty: string;
  accepted?: boolean;
}

export interface JobFlowAgent {
  name: string;
  role: 'poster' | 'worker';
  reputation: number; // 0-1000000
  tier: string;
  active: boolean;
}

export interface JobFlowVotes {
  voters: { approve: boolean | null }[];
  verdict: 'pending' | 'approved' | 'rejected';
}

export type JobFlowEvent =
  | { type: 'posted'; bounty: string }
  | { type: 'assigned' }
  | { type: 'submitted' }
  | { type: 'vote'; approve: boolean }
  | { type: 'resolved'; accepted: boolean };

interface JobFlowVizProps {
  job: JobFlowJobState;
  agents: [JobFlowAgent, JobFlowAgent]; // [poster, worker]
  votes: JobFlowVotes;
  events: JobFlowEvent[];
  jobTitle?: string;
}

/* ── Particle animation state ── */

interface Particle {
  fromX: number;
  fromY: number;
  toX: number;
  toY: number;
  cpX: number;
  cpY: number;
  startTime: number;
  duration: number;
  color: string;
  glowColor: string;
  size: number;
}

/* ── Resolved CSS variable cache ── */

let _resolved: Record<string, string> = {};
function cssVar(name: string): string {
  if (_resolved[name]) return _resolved[name];
  const v = getComputedStyle(document.documentElement).getPropertyValue(name).trim();
  _resolved[name] = v || '#888';
  return _resolved[name];
}

/* ── Helper: lerp ── */
function lerp(a: number, b: number, t: number): number {
  return a + (b - a) * t;
}

/* ── Helper: quadratic bezier position ── */
function quadBezier(
  x0: number, y0: number,
  cx: number, cy: number,
  x1: number, y1: number,
  t: number,
): [number, number] {
  const u = 1 - t;
  return [
    u * u * x0 + 2 * u * t * cx + t * t * x1,
    u * u * y0 + 2 * u * t * cy + t * t * y1,
  ];
}

/* ── Helper: draw rounded rect ── */
function drawRoundedRect(
  ctx: CanvasRenderingContext2D,
  x: number, y: number,
  w: number, h: number,
  r: number,
): void {
  ctx.beginPath();
  ctx.roundRect(x, y, w, h, r);
}

/* ── Helper: wrap text into lines ── */
function wrapText(ctx: CanvasRenderingContext2D, text: string, maxWidth: number): string[] {
  const words = text.split(' ');
  const lines: string[] = [];
  let current = '';
  for (const word of words) {
    const test = current ? `${current} ${word}` : word;
    if (ctx.measureText(test).width > maxWidth && current) {
      lines.push(current);
      current = word;
    } else {
      current = test;
    }
  }
  if (current) lines.push(current);
  return lines;
}

/* ── Layout computation ── */

interface Layout {
  centerX: number;
  alphaX: number;
  alphaY: number;
  jobX: number;
  jobY: number;
  jobW: number;
  jobH: number;
  betaX: number;
  betaY: number;
  valX: number;
  valY: number;
  nodeW: number;
  nodeH: number;
  offsetX: number;
}

function getLayout(w: number, h: number): Layout {
  const centerX = w * 0.5;
  const nodeW = Math.max(120, Math.min(140, w * 0.32));
  const nodeH = Math.max(60, Math.min(72, h * 0.14));
  const jobW = Math.max(140, Math.min(180, w * 0.4));
  const jobH = Math.max(72, Math.min(84, h * 0.16));
  const offsetX = w * 0.22;

  return {
    centerX,
    alphaX: centerX,
    alphaY: h * 0.13,
    jobX: centerX,
    jobY: h * 0.46,
    jobW,
    jobH,
    betaX: centerX - offsetX,
    betaY: h * 0.78,
    valX: centerX + offsetX,
    valY: h * 0.78,
    nodeW,
    nodeH,
    offsetX,
  };
}

/* ── Tier color ── */
function tierColor(tier: string): string {
  const t = tier.toLowerCase();
  if (t.includes('elite') || t.includes('diamond')) return cssVar('--bone-bright');
  if (t.includes('gold')) return cssVar('--warning');
  if (t.includes('silver')) return cssVar('--dream-bright');
  if (t.includes('standard') || t.includes('bronze')) return cssVar('--rose');
  return cssVar('--text-dim');
}

/* ── Job border color by state ── */
function jobBorderColor(state: JobFlowJobState['state'], accepted?: boolean): string {
  if (state === 'resolved') return accepted ? cssVar('--success') : cssVar('--warning');
  if (state === 'assigned' || state === 'submitted') return cssVar('--rose');
  if (state === 'funded') return cssVar('--bone-dim');
  return cssVar('--glass-border');
}

/* ── Draw agent node ── */
function drawAgentNode(
  ctx: CanvasRenderingContext2D,
  lay: Layout,
  agent: JobFlowAgent,
  cx: number,
  cy: number,
  isActive: boolean,
  targetRepFraction: number, // 0-1 for animation
): void {
  const hw = lay.nodeW / 2;
  const hh = lay.nodeH / 2;
  const x = cx - hw;
  const y = cy - hh;
  const r = 6;

  // Background
  drawRoundedRect(ctx, x, y, lay.nodeW, lay.nodeH, r);
  ctx.fillStyle = 'rgba(18,16,26,0.9)';
  ctx.fill();

  // Border
  ctx.strokeStyle = isActive ? cssVar('--rose-glow') : cssVar('--glass-border');
  ctx.lineWidth = isActive ? 1.5 : 1;
  ctx.stroke();

  // Glow when active
  if (isActive) {
    ctx.shadowColor = cssVar('--rose-glow');
    ctx.shadowBlur = 12;
    ctx.stroke();
    ctx.shadowBlur = 0;
  }

  // Name
  ctx.fillStyle = cssVar('--text-strong');
  ctx.font = `bold 13px "JetBrains Mono", monospace`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(agent.name, cx, y + 13);

  // Role dot + label
  const roleDotX = cx - 28;
  const roleDotY = y + 28;
  ctx.beginPath();
  ctx.arc(roleDotX, roleDotY, 4, 0, Math.PI * 2);
  ctx.fillStyle = cssVar('--rose');
  ctx.fill();

  ctx.fillStyle = cssVar('--text-ghost');
  ctx.font = `9px "JetBrains Mono", monospace`;
  ctx.textAlign = 'left';
  ctx.fillText(agent.role, roleDotX + 9, roleDotY + 1);

  // Rep bar background
  const barX = cx - 40;
  const barY = y + 40;
  const barW = 80;
  const barH = 3;
  ctx.fillStyle = 'rgba(255,255,255,0.06)';
  ctx.fillRect(barX, barY, barW, barH);

  // Rep bar fill
  const repFill = Math.max(0, Math.min(1, targetRepFraction));
  if (repFill > 0) {
    ctx.fillStyle = tierColor(agent.tier);
    ctx.fillRect(barX, barY, barW * repFill, barH);
  }

  // Rep number
  const repK = agent.reputation >= 1000 ? `${Math.round(agent.reputation / 1000)}k` : String(agent.reputation);
  ctx.fillStyle = cssVar('--text-dim');
  ctx.font = `10px "JetBrains Mono", monospace`;
  ctx.textAlign = 'left';
  ctx.fillText(repK, barX + barW + 5, barY + 2);

  // Tier label
  ctx.fillStyle = cssVar('--text-ghost');
  ctx.font = `9px "JetBrains Mono", monospace`;
  ctx.textAlign = 'center';
  ctx.fillText(agent.tier, cx, y + lay.nodeH - 8);
}

/* ── Draw job node ── */
function drawJobNode(
  ctx: CanvasRenderingContext2D,
  lay: Layout,
  job: JobFlowJobState,
  escrowFill: number,
  jobTitle: string | undefined,
): void {
  const hw = lay.jobW / 2;
  const hh = lay.jobH / 2;
  const x = lay.jobX - hw;
  const y = lay.jobY - hh;
  const r = 8;

  // Outer border (state-dependent glow)
  const outerColor = jobBorderColor(job.state, job.accepted);
  drawRoundedRect(ctx, x - 1, y - 1, lay.jobW + 2, lay.jobH + 2, r + 1);
  ctx.strokeStyle = outerColor;
  ctx.lineWidth = 1.5;
  ctx.stroke();

  if (job.state !== 'waiting') {
    ctx.shadowColor = outerColor;
    ctx.shadowBlur = 14;
    ctx.stroke();
    ctx.shadowBlur = 0;
  }

  // Background
  drawRoundedRect(ctx, x, y, lay.jobW, lay.jobH, r);
  ctx.fillStyle = 'rgba(18,16,26,0.95)';
  ctx.fill();

  // Inner border
  ctx.strokeStyle = cssVar('--glass-border');
  ctx.lineWidth = 1;
  ctx.stroke();

  // Green wash on resolved+accepted
  if (job.state === 'resolved' && job.accepted) {
    ctx.fillStyle = 'rgba(138,156,134,0.06)';
    ctx.fill();
  }

  // Bounty text (largest)
  ctx.fillStyle = cssVar('--bone-bright');
  ctx.font = `bold 16px "JetBrains Mono", monospace`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(job.bounty, lay.jobX, y + 18);

  // Escrow bar
  const escrowBarX = x + 12;
  const escrowBarY = y + 34;
  const escrowBarW = lay.jobW - 24;
  const escrowBarH = 6;
  ctx.fillStyle = 'rgba(255,255,255,0.06)';
  ctx.fillRect(escrowBarX, escrowBarY, escrowBarW, escrowBarH);

  if (escrowFill > 0) {
    ctx.fillStyle = cssVar('--bone');
    ctx.fillRect(escrowBarX, escrowBarY, escrowBarW * escrowFill, escrowBarH);
  }

  // State label
  ctx.fillStyle = cssVar('--text-ghost');
  ctx.font = `9px "JetBrains Mono", monospace`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(job.state.toUpperCase(), lay.jobX, y + lay.jobH - 10);

  // Job title to the right of the node
  if (jobTitle) {
    ctx.font = `italic 11px "JetBrains Mono", monospace`;
    ctx.fillStyle = cssVar('--text-dim');
    ctx.textAlign = 'left';
    const titleX = lay.jobX + hw + 14;
    const maxTitleW = Math.max(60, lay.jobX + lay.jobW * 1.5 - titleX - 8);
    const lines = wrapText(ctx, jobTitle, maxTitleW);
    lines.slice(0, 2).forEach((line, i) => {
      ctx.fillText(line, titleX, y + 18 + i * 16);
    });
  }
}

/* ── Draw validator cluster ── */
function drawValidatorCluster(
  ctx: CanvasRenderingContext2D,
  lay: Layout,
  votes: JobFlowVotes,
  opacity: number,
): void {
  if (opacity <= 0) return;

  ctx.globalAlpha = opacity;

  const cw = 130;
  const ch = 68;
  const x = lay.valX - cw / 2;
  const y = lay.valY - ch / 2;
  const r = 6;

  // Background
  drawRoundedRect(ctx, x, y, cw, ch, r);
  ctx.fillStyle = 'rgba(18,16,26,0.9)';
  ctx.fill();
  ctx.strokeStyle = cssVar('--glass-border');
  ctx.lineWidth = 1;
  ctx.stroke();

  // Vote circles
  const circleY = y + 22;
  const startX = x + cw / 2 - 28;
  const spacing = 22;

  votes.voters.slice(0, 3).forEach((voter, i) => {
    const cx2 = startX + i * spacing;
    ctx.beginPath();
    ctx.arc(cx2, circleY, 9, 0, Math.PI * 2);

    if (voter.approve === true) {
      ctx.fillStyle = cssVar('--success');
      ctx.fill();
      ctx.fillStyle = cssVar('--bg-void');
      ctx.font = `bold 10px "JetBrains Mono", monospace`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText('✓', cx2, circleY);
    } else if (voter.approve === false) {
      ctx.fillStyle = cssVar('--warning');
      ctx.fill();
      ctx.fillStyle = cssVar('--bg-void');
      ctx.font = `bold 10px "JetBrains Mono", monospace`;
      ctx.textAlign = 'center';
      ctx.textBaseline = 'middle';
      ctx.fillText('✗', cx2, circleY);
    } else {
      ctx.strokeStyle = cssVar('--text-ghost');
      ctx.lineWidth = 1;
      ctx.stroke();
    }
  });

  // Divider
  ctx.strokeStyle = cssVar('--border-soft');
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.moveTo(x + 10, y + 38);
  ctx.lineTo(x + cw - 10, y + 38);
  ctx.stroke();

  // Verdict
  const verdictText = votes.verdict === 'approved' ? 'APPROVED' :
    votes.verdict === 'rejected' ? 'REJECTED' : 'PENDING';
  const verdictColor = votes.verdict === 'approved' ? cssVar('--success') :
    votes.verdict === 'rejected' ? cssVar('--warning') : cssVar('--text-dim');

  ctx.fillStyle = verdictColor;
  ctx.font = `bold 11px "JetBrains Mono", monospace`;
  ctx.textAlign = 'center';
  ctx.textBaseline = 'middle';
  ctx.fillText(verdictText, lay.valX, y + 54);

  ctx.globalAlpha = 1;
}

/* ── Draw edges ── */
function drawEdges(
  ctx: CanvasRenderingContext2D,
  lay: Layout,
  showValidators: boolean,
  flashEdge: boolean,
  validatorOpacity: number,
): void {
  const edgeColor = cssVar('--border-soft');
  const flashColor = cssVar('--rose-glow');

  // Alpha bottom → Job top (vertical bezier)
  const alphaBottom = lay.alphaY + lay.nodeH / 2;
  const jobTop = lay.jobY - lay.jobH / 2;
  const midY1 = (alphaBottom + jobTop) / 2;

  ctx.beginPath();
  ctx.moveTo(lay.alphaX, alphaBottom);
  ctx.bezierCurveTo(lay.alphaX, midY1 - 10, lay.alphaX, midY1 + 10, lay.alphaX, jobTop);
  ctx.strokeStyle = edgeColor;
  ctx.lineWidth = 1;
  ctx.stroke();

  // Job bottom → Beta top (angled bezier)
  const jobBottom = lay.jobY + lay.jobH / 2;
  const betaTop = lay.betaY - lay.nodeH / 2;
  const midY2 = (jobBottom + betaTop) / 2;

  ctx.beginPath();
  ctx.moveTo(lay.jobX, jobBottom);
  ctx.bezierCurveTo(lay.jobX, midY2, lay.betaX, midY2, lay.betaX, betaTop);
  ctx.strokeStyle = flashEdge ? flashColor : edgeColor;
  ctx.lineWidth = flashEdge ? 2 : 1;
  ctx.stroke();

  if (flashEdge) {
    ctx.shadowColor = flashColor;
    ctx.shadowBlur = 10;
    ctx.stroke();
    ctx.shadowBlur = 0;
  }

  // Beta → Validators (horizontal line, only with cluster)
  if (showValidators && validatorOpacity > 0) {
    ctx.globalAlpha = validatorOpacity;
    ctx.beginPath();
    ctx.moveTo(lay.betaX + lay.nodeW / 2, lay.betaY);
    ctx.lineTo(lay.valX - 65, lay.valY);
    ctx.strokeStyle = edgeColor;
    ctx.lineWidth = 1;
    ctx.stroke();
    ctx.globalAlpha = 1;
  }
}

/* ── Component ── */

export default function JobFlowViz({
  job,
  agents,
  votes,
  events,
  jobTitle,
}: JobFlowVizProps) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  const rafRef = useRef(0);

  // Animation refs (no React re-renders)
  const particlesRef = useRef<Particle[]>([]);
  const escrowFillRef = useRef(0);
  const escrowTargetRef = useRef(0);
  const escrowAnimStartRef = useRef(0);
  const escrowAnimDurRef = useRef(0);
  const showValidatorsRef = useRef(false);
  const validatorOpacityRef = useRef(0);
  const validatorFadeStartRef = useRef(0);
  const flashEdgeRef = useRef(0); // timestamp until flash ends
  const activeBorderRef = useRef<'alpha' | 'beta' | null>(null);
  const activeBorderEndRef = useRef(0);
  const prevEventsRef = useRef(0);
  const repAnimRef = useRef({ current: agents[1].reputation / 1000000, target: agents[1].reputation / 1000000, start: 0, dur: 0 });

  // Props that drawing needs, via refs for stable RAF callbacks
  const jobRef = useRef(job);
  const agentsRef = useRef(agents);
  const votesRef = useRef(votes);
  const jobTitleRef = useRef(jobTitle);
  useEffect(() => { jobRef.current = job; }, [job]);
  useEffect(() => { agentsRef.current = agents; }, [agents]);
  useEffect(() => { votesRef.current = votes; }, [votes]);
  useEffect(() => { jobTitleRef.current = jobTitle; }, [jobTitle]);

  /* ── Spawn particle ── */
  const spawnParticle = useCallback((
    fromX: number, fromY: number,
    toX: number, toY: number,
    color: string,
    glowColor: string,
    duration: number,
    size = 4,
  ) => {
    const cpX = (fromX + toX) / 2;
    const cpY = Math.min(fromY, toY) - 20;
    particlesRef.current.push({
      fromX, fromY, toX, toY, cpX, cpY,
      startTime: performance.now(),
      duration,
      color,
      glowColor,
      size,
    });
  }, []);

  /* ── React to new events ── */
  useEffect(() => {
    if (events.length <= prevEventsRef.current) {
      prevEventsRef.current = events.length;
      return;
    }

    const newEvents = events.slice(prevEventsRef.current);
    prevEventsRef.current = events.length;

    const canvas = canvasRef.current;
    if (!canvas) return;
    const rect = canvas.getBoundingClientRect();
    const lay = getLayout(rect.width, rect.height);

    for (const ev of newEvents) {
      if (ev.type === 'posted') {
        // Alpha → Job particle
        spawnParticle(
          lay.alphaX, lay.alphaY + lay.nodeH / 2,
          lay.jobX, lay.jobY - lay.jobH / 2,
          cssVar('--bone'),
          cssVar('--bone-bright'),
          800,
          4,
        );
        // Animate escrow fill 0→1
        escrowFillRef.current = 0;
        escrowTargetRef.current = 1;
        escrowAnimStartRef.current = performance.now();
        escrowAnimDurRef.current = 400;
      } else if (ev.type === 'assigned') {
        // Flash Job→Beta edge, activate Beta border
        flashEdgeRef.current = performance.now() + 300;
        activeBorderRef.current = 'beta';
        activeBorderEndRef.current = performance.now() + 1000;
      } else if (ev.type === 'submitted') {
        // Show validators
        showValidatorsRef.current = true;
        validatorFadeStartRef.current = performance.now();
      } else if (ev.type === 'resolved') {
        if (ev.accepted) {
          // Job → Beta particle
          spawnParticle(
            lay.jobX, lay.jobY + lay.jobH / 2,
            lay.betaX, lay.betaY - lay.nodeH / 2,
            cssVar('--bone'),
            cssVar('--bone-bright'),
            800,
            5,
          );
          // Drain escrow
          escrowTargetRef.current = 0;
          escrowAnimStartRef.current = performance.now();
          escrowAnimDurRef.current = 600;
          // Animate Beta rep bar
          const newRepFrac = Math.min(1, (agentsRef.current[1].reputation + 50000) / 1000000);
          repAnimRef.current = {
            current: repAnimRef.current.current,
            target: newRepFrac,
            start: performance.now(),
            dur: 800,
          };
        }
      }
    }

    // Kick RAF
    cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(draw);
  }, [events, spawnParticle]); // eslint-disable-line react-hooks/exhaustive-deps

  /* ── Main draw function ── */
  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    canvas.width = rect.width * dpr;
    canvas.height = rect.height * dpr;
    ctx.scale(dpr, dpr);

    const w = rect.width;
    const h = rect.height;
    const lay = getLayout(w, h);
    const now = performance.now();

    // Clear
    ctx.clearRect(0, 0, w, h);
    ctx.fillStyle = cssVar('--bg-void');
    ctx.fillRect(0, 0, w, h);

    const currentJob = jobRef.current;
    const currentAgents = agentsRef.current;
    const currentVotes = votesRef.current;

    // Update escrow fill animation
    if (escrowAnimDurRef.current > 0) {
      const p = Math.min(1, (now - escrowAnimStartRef.current) / escrowAnimDurRef.current);
      escrowFillRef.current = lerp(
        escrowFillRef.current,
        escrowTargetRef.current,
        p,
      );
      if (p >= 1) escrowAnimDurRef.current = 0;
    } else {
      // Snap to target state from props
      if (currentJob.state === 'funded' || currentJob.state === 'assigned' || currentJob.state === 'submitted') {
        escrowFillRef.current = 1;
      } else if (currentJob.state === 'resolved' && currentJob.accepted) {
        escrowFillRef.current = 0;
      }
    }

    // Update validator fade
    if (showValidatorsRef.current && validatorOpacityRef.current < 1) {
      const elapsed = now - validatorFadeStartRef.current;
      validatorOpacityRef.current = Math.min(1, elapsed / 300);
    }

    // Update active border
    const flashEdge = now < flashEdgeRef.current;
    const activeNode = now < activeBorderEndRef.current ? activeBorderRef.current : null;

    // Update rep bar animation
    const rep = repAnimRef.current;
    if (rep.dur > 0) {
      const p = Math.min(1, (now - rep.start) / rep.dur);
      rep.current = lerp(rep.current, rep.target, p);
      if (p >= 1) rep.dur = 0;
    } else {
      rep.current = currentAgents[1].reputation / 1000000;
      rep.target = rep.current;
    }

    // ── Edges ──
    drawEdges(
      ctx, lay,
      showValidatorsRef.current,
      flashEdge,
      validatorOpacityRef.current,
    );

    // ── Alpha node (poster) ──
    drawAgentNode(
      ctx, lay,
      currentAgents[0],
      lay.alphaX, lay.alphaY,
      activeNode === 'alpha' || currentAgents[0].active,
      currentAgents[0].reputation / 1000000,
    );

    // ── Job node ──
    drawJobNode(ctx, lay, currentJob, escrowFillRef.current, jobTitleRef.current);

    // ── Beta node (worker) ──
    drawAgentNode(
      ctx, lay,
      currentAgents[1],
      lay.betaX, lay.betaY,
      activeNode === 'beta' || currentAgents[1].active,
      rep.current,
    );

    // ── Validator cluster ──
    drawValidatorCluster(ctx, lay, currentVotes, validatorOpacityRef.current);

    // ── Particles ──
    const alive: Particle[] = [];
    for (const p of particlesRef.current) {
      const elapsed = now - p.startTime;
      const t = Math.min(elapsed / p.duration, 1);
      if (t >= 1) continue;
      alive.push(p);

      const [px, py] = quadBezier(p.fromX, p.fromY, p.cpX, p.cpY, p.toX, p.toY, t);
      const alpha = t < 0.1 ? t / 0.1 : t > 0.9 ? (1 - t) / 0.1 : 1;

      // Glow pass
      ctx.shadowColor = p.glowColor;
      ctx.shadowBlur = 12;
      ctx.beginPath();
      ctx.arc(px, py, p.size, 0, Math.PI * 2);
      ctx.fillStyle = p.color;
      ctx.globalAlpha = alpha * 0.85;
      ctx.fill();
      ctx.shadowBlur = 0;
      ctx.globalAlpha = 1;
    }
    particlesRef.current = alive;

    // Continue RAF if anything is animated
    const hasEscrowAnim = escrowAnimDurRef.current > 0;
    const hasRepAnim = rep.dur > 0;
    const hasValidatorFade = showValidatorsRef.current && validatorOpacityRef.current < 1;
    const hasFlash = flashEdge || now < activeBorderEndRef.current;

    if (alive.length > 0 || hasEscrowAnim || hasRepAnim || hasValidatorFade || hasFlash) {
      rafRef.current = requestAnimationFrame(draw);
    }
  }, []); // no deps — reads from refs

  /* ── Start/restart draw on prop changes ── */
  useEffect(() => {
    cancelAnimationFrame(rafRef.current);
    rafRef.current = requestAnimationFrame(draw);

    const ro = new ResizeObserver(() => {
      cancelAnimationFrame(rafRef.current);
      rafRef.current = requestAnimationFrame(draw);
    });
    if (canvasRef.current) ro.observe(canvasRef.current);

    return () => {
      cancelAnimationFrame(rafRef.current);
      ro.disconnect();
    };
  }, [draw, job, agents, votes]);

  return (
    <div className="job-flow-viz">
      <canvas ref={canvasRef} />
    </div>
  );
}
