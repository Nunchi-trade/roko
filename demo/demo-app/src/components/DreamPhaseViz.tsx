import { useCallback, useEffect, useRef, useState } from 'react';
import { useLiveApi } from '../hooks/useLiveApi';

interface DreamPhase {
  name: string;
  status: string;
  episodes_processed: number;
  clusters_formed: number;
  knowledge_entries_written: number;
  playbooks_created: number;
  duration_secs: number;
  trend: number[];
}

interface DreamJournal {
  last_cycle: string;
  cycle_count: number;
  phases: DreamPhase[];
}

const PHASE_SEQUENCE = ['Hypnagogia', 'NREM', 'REM', 'Integration'] as const;

const PHASE_STYLES: Record<string, { color: string; bg: string; border: string; glow: string }> = {
  Hypnagogia: {
    color: '#9A8AB8',
    bg: 'rgba(154,138,184,0.06)',
    border: 'rgba(154,138,184,0.15)',
    glow: 'rgba(154,138,184,0.5)',
  },
  NREM: {
    color: '#7A8AA8',
    bg: 'rgba(122,138,168,0.06)',
    border: 'rgba(122,138,168,0.15)',
    glow: 'rgba(122,138,168,0.5)',
  },
  REM: {
    color: '#CC90A8',
    bg: 'rgba(204,144,168,0.06)',
    border: 'rgba(204,144,168,0.15)',
    glow: 'rgba(204,144,168,0.5)',
  },
  Integration: {
    color: '#8A9C86',
    bg: 'rgba(138,156,134,0.06)',
    border: 'rgba(138,156,134,0.15)',
    glow: 'rgba(138,156,134,0.5)',
  },
};

function hexToRgba(hex: string, alpha: number): string {
  const normalized = hex.replace('#', '');
  const expanded =
    normalized.length === 3
      ? normalized
          .split('')
          .map((segment) => `${segment}${segment}`)
          .join('')
      : normalized;

  const r = Number.parseInt(expanded.slice(0, 2), 16);
  const g = Number.parseInt(expanded.slice(2, 4), 16);
  const b = Number.parseInt(expanded.slice(4, 6), 16);

  if (Number.isNaN(r) || Number.isNaN(g) || Number.isNaN(b)) return hex;
  return `rgba(${r}, ${g}, ${b}, ${alpha})`;
}

function formatCycleTime(iso: string): string {
  const date = new Date(iso);
  return Number.isNaN(date.getTime())
    ? iso
    : date.toLocaleString(undefined, {
        month: 'short',
        day: 'numeric',
        hour: 'numeric',
        minute: '2-digit',
      });
}

function PhaseSparkline({
  data,
  color,
  width = 132,
  height = 34,
}: {
  data: number[];
  color: string;
  width?: number;
  height?: number;
}) {
  const canvasRef = useRef<HTMLCanvasElement>(null);

  const draw = useCallback(() => {
    const canvas = canvasRef.current;
    if (!canvas) return;

    const ctx = canvas.getContext('2d');
    if (!ctx) return;

    const dpr = window.devicePixelRatio || 1;
    const rect = canvas.getBoundingClientRect();
    const resolvedWidth = rect.width || width;
    const resolvedHeight = rect.height || height;

    canvas.width = resolvedWidth * dpr;
    canvas.height = resolvedHeight * dpr;
    ctx.scale(dpr, dpr);

    ctx.clearRect(0, 0, resolvedWidth, resolvedHeight);

    const pad = 2;
    const plotW = resolvedWidth - pad * 2;
    const plotH = resolvedHeight - pad * 2;

    if (data.length < 2) {
      ctx.strokeStyle = 'rgba(255,255,255,0.08)';
      ctx.lineWidth = 1;
      ctx.setLineDash([3, 4]);
      ctx.beginPath();
      ctx.moveTo(pad, resolvedHeight / 2);
      ctx.lineTo(resolvedWidth - pad, resolvedHeight / 2);
      ctx.stroke();
      ctx.setLineDash([]);
      return;
    }

    const min = Math.min(...data);
    const max = Math.max(...data);
    const range = max - min || 1;

    ctx.beginPath();
    for (let i = 0; i < data.length; i += 1) {
      const x = pad + (i / (data.length - 1)) * plotW;
      const y = pad + plotH * (1 - (data[i] - min) / range);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.lineTo(pad + plotW, pad + plotH);
    ctx.lineTo(pad, pad + plotH);
    ctx.closePath();
    ctx.fillStyle = hexToRgba(color, 0.12);
    ctx.fill();

    ctx.beginPath();
    ctx.strokeStyle = color;
    ctx.lineWidth = 1.5;
    ctx.lineJoin = 'round';
    ctx.lineCap = 'round';
    for (let i = 0; i < data.length; i += 1) {
      const x = pad + (i / (data.length - 1)) * plotW;
      const y = pad + plotH * (1 - (data[i] - min) / range);
      if (i === 0) ctx.moveTo(x, y);
      else ctx.lineTo(x, y);
    }
    ctx.stroke();

    const lastX = pad + plotW;
    const lastY = pad + plotH * (1 - (data[data.length - 1] - min) / range);
    ctx.beginPath();
    ctx.arc(lastX, lastY, 2, 0, Math.PI * 2);
    ctx.fillStyle = color;
    ctx.fill();
  }, [data, color, width, height]);

  useEffect(() => {
    draw();
    const observer = new ResizeObserver(draw);
    if (canvasRef.current) observer.observe(canvasRef.current);
    return () => observer.disconnect();
  }, [draw]);

  return (
    <canvas
      ref={canvasRef}
      aria-hidden="true"
      style={{ width: '100%', height, display: 'block' }}
    />
  );
}

function PhaseStat({ label, value }: { label: string; value: string | number }) {
  return (
    <div style={{ display: 'flex', justifyContent: 'space-between', gap: 8 }}>
      <span
        style={{
          fontFamily: 'var(--mono)',
          fontSize: '0.6rem',
          letterSpacing: '0.06em',
          textTransform: 'uppercase',
          color: 'var(--text-dim)',
        }}
      >
        {label}
      </span>
      <span
        style={{
          fontFamily: 'var(--mono)',
          fontSize: '0.7rem',
          fontWeight: 500,
          color: 'var(--text-primary)',
        }}
      >
        {value}
      </span>
    </div>
  );
}

function PhaseArrow({ color }: { color: string }) {
  return (
    <div
      aria-hidden="true"
      style={{
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        minWidth: 24,
        color,
        opacity: 0.55,
        fontFamily: 'var(--mono)',
        fontSize: 14,
        letterSpacing: '0.08em',
      }}
    >
      {'->'}
    </div>
  );
}

export default function DreamPhaseViz() {
  const { get } = useLiveApi();
  const [journal, setJournal] = useState<DreamJournal | null>(null);

  useEffect(() => {
    let cancelled = false;

    void get<DreamJournal>('/api/dream/journal')
      .then((data) => {
        if (!cancelled) setJournal(data);
      })
      .catch(() => {
        if (!cancelled) setJournal(null);
      });

    return () => {
      cancelled = true;
    };
  }, [get]);

  const phasesByName = new Map(journal?.phases.map((phase) => [phase.name, phase] as const) ?? []);
  const cycleCount = journal?.cycle_count ?? 0;
  const lastCycle = journal?.last_cycle ?? '';
  const completedPhases = journal?.phases.filter((phase) => phase.status === 'completed').length ?? 0;
  const hasJournal = journal !== null;
  const cycleCountText = hasJournal ? `${cycleCount} cycles completed` : 'loading dream journal';
  const completionText = hasJournal ? `${completedPhases}/4 phases complete` : 'awaiting journal data';

  return (
    <div style={{ display: 'flex', flexDirection: 'column', gap: 12, width: '100%' }}>
      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          alignItems: 'baseline',
          gap: 12,
          flexWrap: 'wrap',
          padding: '0 4px',
        }}
      >
        <div style={{ display: 'flex', flexDirection: 'column', gap: 2 }}>
          <span
            style={{
              fontFamily: 'var(--mono)',
              fontSize: '0.6rem',
              letterSpacing: '0.12em',
              textTransform: 'uppercase',
              color: 'var(--text-dim)',
            }}
          >
            dream consolidation journal
          </span>
          <span
            style={{
              fontFamily: 'var(--display)',
              fontSize: '0.95rem',
              color: 'var(--text-primary)',
            }}
          >
            {cycleCountText}
          </span>
        </div>
        <div style={{ textAlign: 'right' }}>
          <span
            style={{
              display: 'block',
              fontFamily: 'var(--mono)',
              fontSize: '0.6rem',
              letterSpacing: '0.12em',
              textTransform: 'uppercase',
              color: 'var(--text-dim)',
            }}
          >
            last cycle
          </span>
          <span
            style={{
              fontFamily: 'var(--mono)',
              fontSize: '0.65rem',
              color: 'var(--text-ghost)',
            }}
          >
            {lastCycle ? formatCycleTime(lastCycle) : 'awaiting consolidation'}
          </span>
        </div>
      </div>

      <div
        style={{
          display: 'flex',
          alignItems: 'stretch',
          gap: 0,
          overflowX: 'auto',
          paddingBottom: 2,
        }}
      >
        {PHASE_SEQUENCE.map((phaseName, index) => {
          const style = PHASE_STYLES[phaseName];
          const phase = phasesByName.get(phaseName);
          const ready = hasJournal && Boolean(phase);

          return (
            <div key={phaseName} style={{ display: 'flex', alignItems: 'stretch', flex: '0 0 auto' }}>
              <section
                style={{
                  width: 220,
                  minWidth: 220,
                  borderRadius: 12,
                  border: `1px solid ${style.border}`,
                  background: style.bg,
                  boxShadow: `0 0 0 1px rgba(255,255,255,0.02) inset, 0 12px 28px rgba(0,0,0,0.12)`,
                  padding: '14px 14px 12px',
                  display: 'flex',
                  flexDirection: 'column',
                  gap: 10,
                }}
              >
                <div style={{ display: 'flex', alignItems: 'center', gap: 8 }}>
                  <span
                    style={{
                      width: 6,
                      height: 6,
                      borderRadius: '50%',
                      background: ready && phase?.status === 'completed' ? style.color : 'rgba(255,255,255,0.22)',
                      boxShadow: ready && phase?.status === 'completed' ? `0 0 8px ${style.glow}` : 'none',
                    }}
                  />
                  <div style={{ display: 'flex', flexDirection: 'column', gap: 1 }}>
                    <span
                      style={{
                        fontFamily: 'var(--display)',
                        fontSize: 13,
                        fontWeight: 500,
                        color: style.color,
                        letterSpacing: '0.02em',
                      }}
                    >
                      {phaseName}
                    </span>
                    <span
                      style={{
                        fontFamily: 'var(--mono)',
                        fontSize: '0.58rem',
                        letterSpacing: '0.08em',
                        textTransform: 'uppercase',
                        color: 'var(--text-dim)',
                      }}
                    >
                      {ready ? phase?.status : 'loading'}
                    </span>
                  </div>
                </div>

                <PhaseSparkline
                  data={ready && phase ? phase.trend : []}
                  color={style.color}
                  width={192}
                  height={34}
                />

                <div
                  style={{
                    display: 'flex',
                    flexDirection: 'column',
                    gap: 4,
                    borderTop: '1px solid rgba(255,255,255,0.04)',
                    paddingTop: 8,
                  }}
                >
                  <PhaseStat label="episodes" value={ready && phase ? phase.episodes_processed : 'n/a'} />
                  <PhaseStat label="clusters" value={ready && phase ? phase.clusters_formed : 'n/a'} />
                  <PhaseStat
                    label="knowledge"
                    value={ready && phase ? phase.knowledge_entries_written : 'n/a'}
                  />
                  <PhaseStat label="playbooks" value={ready && phase ? phase.playbooks_created : 'n/a'} />
                  <PhaseStat label="duration" value={ready && phase ? `${phase.duration_secs}s` : 'n/a'} />
                </div>
              </section>

              {index < PHASE_SEQUENCE.length - 1 && <PhaseArrow color={style.color} />}
            </div>
          );
        })}
      </div>

      <div
        style={{
          display: 'flex',
          justifyContent: 'space-between',
          gap: 12,
          flexWrap: 'wrap',
          padding: '0 4px',
          fontFamily: 'var(--mono)',
          fontSize: '0.6rem',
          letterSpacing: '0.08em',
          textTransform: 'uppercase',
          color: 'var(--text-dim)',
        }}
      >
        <span>{completionText}</span>
        <span>canvas sparklines update from the dream journal feed</span>
      </div>
    </div>
  );
}
