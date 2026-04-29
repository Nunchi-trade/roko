import './ScenarioDiagram.css';

// ── Types ────────────────────────────────────────────────────

export type DiagramIcon =
  | 'pipeline'
  | 'race'
  | 'gate'
  | 'grid'
  | 'explore'
  | 'knowledge'
  | 'dream'
  | 'chat'
  | 'transfer'
  | 'chain'
  | 'evm';

export interface ScenarioDiagramProps {
  icon: DiagramIcon;
  accent: string;
  animated?: boolean;
}

// ── Helpers ──────────────────────────────────────────────────

const TEXT_STYLE: React.CSSProperties = {
  fontFamily: 'var(--mono)',
  fontSize: '7px',
  letterSpacing: '0.05em',
  textTransform: 'uppercase' as const,
  fill: 'var(--text-dim)',
  pointerEvents: 'none',
};

const LABEL_STYLE: React.CSSProperties = {
  ...TEXT_STYLE,
  fontSize: '6px',
};

// ── Diagrams ─────────────────────────────────────────────────

function PipelineDiagram({ accent }: { accent: string }) {
  const labels = ['IDEA', 'DRAFT', 'RSCH', 'PLAN', 'EXEC', 'GATE', 'LEARN'];
  const spacing = 26;
  const startX = 16;
  const cy = 50;

  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {labels.map((label, i) => {
        const cx = startX + i * spacing;
        return (
          <g key={label}>
            {i < labels.length - 1 && (
              <line
                className="sd-pipeline-arrow"
                x1={cx + 8}
                y1={cy}
                x2={cx + spacing - 8}
                y2={cy}
                stroke={accent}
                strokeWidth="1"
                strokeOpacity="0.5"
              />
            )}
            <circle
              className="sd-pipeline-node"
              cx={cx}
              cy={cy}
              r="7"
              fill={accent}
              fillOpacity="0.2"
              stroke={accent}
              strokeWidth="1"
            />
            <text
              x={cx}
              y={cy + 18}
              textAnchor="middle"
              style={LABEL_STYLE}
            >
              {label}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

function RaceDiagram({ accent }: { accent: string }) {
  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* Lanes */}
      <rect x="15" y="30" width="160" height="24" rx="2" fill="var(--surface)" stroke="var(--border)" strokeWidth="0.5" />
      <rect x="15" y="66" width="160" height="24" rx="2" fill="var(--surface)" stroke="var(--border)" strokeWidth="0.5" />

      {/* Lane labels */}
      <text x="12" y="28" style={LABEL_STYLE} textAnchor="end">Naive</text>
      <text x="12" y="64" style={LABEL_STYLE} textAnchor="end">Cascade</text>

      {/* Finish line */}
      <line x1="175" y1="26" x2="175" y2="94" stroke="var(--text-dim)" strokeWidth="1" strokeDasharray="3 2" strokeOpacity="0.5" />
      <text x="175" y="104" textAnchor="middle" style={LABEL_STYLE}>Finish</text>

      {/* Racing dots */}
      <circle className="sd-race-dot-naive" cy="42" r="4" fill="var(--text-dim)" />
      <circle className="sd-race-dot-cascade" cy="78" r="4" fill={accent} />
    </svg>
  );
}

function GateDiagram({ accent }: { accent: string }) {
  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* Shield shape */}
      <path
        className="sd-gate-shield"
        d="M100 20 L130 35 L130 65 Q130 90 100 100 Q70 90 70 65 L70 35 Z"
        fill={accent}
        fillOpacity="0.3"
        stroke={accent}
        strokeWidth="1.5"
      />

      {/* Check / X inside shield */}
      <path
        d="M88 58 L96 66 L112 50"
        stroke="var(--text-strong)"
        strokeWidth="2"
        strokeLinecap="round"
        strokeLinejoin="round"
        fill="none"
      />

      {/* Retry arrow loop (circular arc around shield) */}
      <path
        className="sd-gate-retry"
        d="M55 60 A50 50 0 1 1 55 62"
        stroke={accent}
        strokeWidth="1"
        fill="none"
        strokeLinecap="round"
        strokeOpacity="0.6"
      />

      {/* Arrow tip on retry */}
      <polygon
        points="50,56 55,60 50,64"
        fill={accent}
        fillOpacity="0.6"
      />

      <text x="100" y="115" textAnchor="middle" style={LABEL_STYLE}>Verify &amp; Retry</text>
    </svg>
  );
}

function GridDiagram({ accent }: { accent: string }) {
  const providers = ['Claude', 'GPT-4', 'Gemini', 'Ollama'];
  const positions = [
    { x: 30, y: 25 },
    { x: 110, y: 25 },
    { x: 30, y: 65 },
    { x: 110, y: 65 },
  ];

  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {providers.map((name, i) => (
        <g key={name} className="sd-grid-box">
          <rect
            x={positions[i].x}
            y={positions[i].y}
            width="60"
            height="28"
            rx="3"
            fill={accent}
            fillOpacity="0.1"
            stroke={accent}
            strokeWidth="0.8"
          />
          <text
            x={positions[i].x + 30}
            y={positions[i].y + 17}
            textAnchor="middle"
            style={TEXT_STYLE}
          >
            {name}
          </text>
        </g>
      ))}
    </svg>
  );
}

function ExploreDiagram({ accent }: { accent: string }) {
  const branches = [
    { label: 'Config', angle: -60, len: 38 },
    { label: 'Knowledge', angle: -20, len: 42 },
    { label: 'Learning', angle: 20, len: 42 },
    { label: 'Workspace', angle: 60, len: 38 },
  ];

  const cx = 100;
  const cy = 60;

  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* Center node */}
      <circle cx={cx} cy={cy} r="10" fill={accent} fillOpacity="0.25" stroke={accent} strokeWidth="1.5" />
      <text x={cx} y={cy + 3} textAnchor="middle" style={{ ...LABEL_STYLE, fontSize: '5px', fill: 'var(--text-strong)' }}>
        ROKO
      </text>

      {/* Branches */}
      {branches.map((b, i) => {
        const rad = (b.angle * Math.PI) / 180;
        const ex = cx + Math.cos(rad) * b.len;
        const ey = cy + Math.sin(rad) * b.len;

        return (
          <g key={b.label}>
            <line
              className="sd-explore-branch"
              x1={cx}
              y1={cy}
              x2={ex}
              y2={ey}
              stroke={accent}
              strokeWidth="1"
              strokeOpacity="0.6"
              style={{ animationDelay: `${i * 0.3}s` }}
            />
            <circle
              className="sd-explore-endpoint"
              cx={ex}
              cy={ey}
              r="4"
              fill={accent}
              fillOpacity="0.4"
              stroke={accent}
              strokeWidth="0.8"
              style={{ animationDelay: `${i * 0.3 + 0.2}s` }}
            />
            <text
              x={ex}
              y={ey + 12}
              textAnchor="middle"
              style={LABEL_STYLE}
            >
              {b.label}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

function KnowledgeDiagram({ accent }: { accent: string }) {
  const bars = [
    { label: 'Run 1', height: 30 },
    { label: 'Run 2', height: 50 },
    { label: 'Run 3', height: 70 },
  ];
  const baseY = 95;
  const barWidth = 28;
  const gap = 16;
  const totalW = bars.length * barWidth + (bars.length - 1) * gap;
  const startX = (200 - totalW) / 2;

  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* Baseline */}
      <line x1={startX - 5} y1={baseY} x2={startX + totalW + 5} y2={baseY} stroke="var(--border)" strokeWidth="0.5" />

      {bars.map((b, i) => {
        const x = startX + i * (barWidth + gap);
        return (
          <g key={b.label}>
            <rect
              className="sd-knowledge-bar"
              x={x}
              y={baseY - b.height}
              width={barWidth}
              height={b.height}
              rx="2"
              fill={accent}
              fillOpacity={0.2 + i * 0.15}
              stroke={accent}
              strokeWidth="0.8"
              style={{ animationDelay: `${i * 0.4}s` }}
            />
            <text
              x={x + barWidth / 2}
              y={baseY + 12}
              textAnchor="middle"
              style={LABEL_STYLE}
            >
              {b.label}
            </text>
          </g>
        );
      })}
    </svg>
  );
}

function DreamDiagram({ accent }: { accent: string }) {
  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* Moon crescent */}
      <circle cx="80" cy="40" r="20" fill={accent} fillOpacity="0.2" />
      <circle cx="88" cy="34" r="18" fill="var(--bg-void)" />

      {/* Stars */}
      {[
        { cx: 120, cy: 22 },
        { cx: 140, cy: 38 },
        { cx: 130, cy: 52 },
      ].map((s, i) => (
        <g key={i} className="sd-dream-star" style={{ animationDelay: `${i * 0.7}s` }}>
          <circle cx={s.cx} cy={s.cy} r="2" fill={accent} />
          <line x1={s.cx - 4} y1={s.cy} x2={s.cx + 4} y2={s.cy} stroke={accent} strokeWidth="0.5" />
          <line x1={s.cx} y1={s.cy - 4} x2={s.cx} y2={s.cy + 4} stroke={accent} strokeWidth="0.5" />
        </g>
      ))}

      {/* Arrow down to knowledge diamond */}
      <line
        className="sd-dream-arrow"
        x1="100"
        y1="70"
        x2="100"
        y2="88"
        stroke={accent}
        strokeWidth="1"
        strokeOpacity="0.6"
        markerEnd="url(#sd-dream-arrowhead)"
      />
      <defs>
        <marker id="sd-dream-arrowhead" markerWidth="6" markerHeight="6" refX="3" refY="3" orient="auto">
          <path d="M0,0 L6,3 L0,6 Z" fill={accent} fillOpacity="0.6" />
        </marker>
      </defs>

      {/* Knowledge diamond */}
      <polygon
        points="100,88 112,98 100,108 88,98"
        fill={accent}
        fillOpacity="0.2"
        stroke={accent}
        strokeWidth="1"
      />
      <text x="100" y="101" textAnchor="middle" style={{ ...LABEL_STYLE, fontSize: '5px' }}>
        K
      </text>
    </svg>
  );
}

function ChatDiagram({ accent }: { accent: string }) {
  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* Speech bubble */}
      <rect
        x="50"
        y="25"
        width="100"
        height="55"
        rx="8"
        fill={accent}
        fillOpacity="0.1"
        stroke={accent}
        strokeWidth="1"
      />
      {/* Tail */}
      <path
        d="M75 80 L65 95 L85 80"
        fill={accent}
        fillOpacity="0.1"
        stroke={accent}
        strokeWidth="1"
        strokeLinejoin="round"
      />

      {/* Typing dots */}
      {[85, 100, 115].map((cx, i) => (
        <circle
          key={i}
          className="sd-chat-dot"
          cx={cx}
          cy="52"
          r="4"
          fill={accent}
          fillOpacity="0.6"
          style={{ animationDelay: `${i * 0.15}s` }}
        />
      ))}
    </svg>
  );
}

function TransferDiagram({ accent }: { accent: string }) {
  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* Agent A circle */}
      <circle cx="50" cy="60" r="18" fill={accent} fillOpacity="0.15" stroke={accent} strokeWidth="1" />
      <text x="50" y="63" textAnchor="middle" style={TEXT_STYLE}>A</text>

      {/* Agent B circle */}
      <circle cx="150" cy="60" r="18" fill={accent} fillOpacity="0.15" stroke={accent} strokeWidth="1" />
      <text x="150" y="63" textAnchor="middle" style={TEXT_STYLE}>B</text>

      {/* Connection lines */}
      <line x1="70" y1="54" x2="130" y2="54" stroke={accent} strokeWidth="0.5" strokeOpacity="0.3" />
      <line x1="70" y1="66" x2="130" y2="66" stroke={accent} strokeWidth="0.5" strokeOpacity="0.3" />

      {/* Data packets */}
      <circle className="sd-transfer-packet-ab" cy="54" r="3" fill={accent} fillOpacity="0.8" />
      <circle className="sd-transfer-packet-ba" cy="66" r="3" fill={accent} fillOpacity="0.8" />

      {/* Arrow tips */}
      <polygon points="132,50 132,58 136,54" fill={accent} fillOpacity="0.4" />
      <polygon points="68,62 68,70 64,66" fill={accent} fillOpacity="0.4" />
    </svg>
  );
}

function ChainDiagram({ accent }: { accent: string }) {
  const blocks = [
    { x: 20, label: '#1' },
    { x: 80, label: '#2' },
    { x: 140, label: '#3' },
  ];

  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {blocks.map((b, i) => (
        <g key={b.label}>
          {/* Block */}
          <g className="sd-chain-block" style={{ animationDelay: `${i * 0.4}s` }}>
            <rect
              x={b.x}
              y="35"
              width="45"
              height="50"
              rx="3"
              fill={accent}
              fillOpacity="0.1"
              stroke={accent}
              strokeWidth="1"
            />
            <text x={b.x + 22.5} y="55" textAnchor="middle" style={TEXT_STYLE}>
              {b.label}
            </text>
            {/* Hash preview */}
            <text x={b.x + 22.5} y="72" textAnchor="middle" style={{ ...LABEL_STYLE, fontSize: '5px', fill: 'var(--text-ghost)' }}>
              0x{(i + 1).toString(16).padStart(2, '0')}..
            </text>
          </g>

          {/* Hash connection line */}
          {i < blocks.length - 1 && (
            <line
              className="sd-chain-hash"
              x1={b.x + 45}
              y1="60"
              x2={blocks[i + 1].x}
              y2="60"
              stroke={accent}
              strokeWidth="1"
              strokeOpacity="0.5"
              style={{ animationDelay: `${i * 0.4 + 0.2}s` }}
            />
          )}
        </g>
      ))}
    </svg>
  );
}

function EvmDiagram({ accent }: { accent: string }) {
  return (
    <svg viewBox="0 0 200 120" fill="none" xmlns="http://www.w3.org/2000/svg">
      {/* Ethereum diamond (center) */}
      <g>
        <polygon
          points="100,15 120,55 100,70 80,55"
          fill={accent}
          fillOpacity="0.15"
          stroke={accent}
          strokeWidth="1"
        />
        <polygon
          points="100,70 120,55 100,45 80,55"
          fill={accent}
          fillOpacity="0.25"
          stroke={accent}
          strokeWidth="0.5"
        />
      </g>

      {/* Streaming blocks */}
      {[0, 1, 2].map((i) => (
        <g key={i} className="sd-evm-block" style={{ animationDelay: `${i * 1}s` }}>
          <rect
            x={60 + i * 25}
            y="82"
            width="18"
            height="14"
            rx="2"
            fill={accent}
            fillOpacity="0.2"
            stroke={accent}
            strokeWidth="0.6"
          />
          <text
            x={69 + i * 25}
            y="92"
            textAnchor="middle"
            style={{ ...LABEL_STYLE, fontSize: '5px' }}
          >
            BLK
          </text>
        </g>
      ))}

      {/* Connection from diamond to blocks */}
      <line x1="100" y1="70" x2="100" y2="82" stroke={accent} strokeWidth="0.5" strokeOpacity="0.4" strokeDasharray="2 2" />
    </svg>
  );
}

// ── Diagram map ──────────────────────────────────────────────

const DIAGRAMS: Record<DiagramIcon, React.FC<{ accent: string }>> = {
  pipeline: PipelineDiagram,
  race: RaceDiagram,
  gate: GateDiagram,
  grid: GridDiagram,
  explore: ExploreDiagram,
  knowledge: KnowledgeDiagram,
  dream: DreamDiagram,
  chat: ChatDiagram,
  transfer: TransferDiagram,
  chain: ChainDiagram,
  evm: EvmDiagram,
};

// ── Main component ───────────────────────────────────────────

export default function ScenarioDiagram({ icon, accent, animated = true }: ScenarioDiagramProps) {
  const Diagram = DIAGRAMS[icon];

  if (!Diagram) return null;

  return (
    <div
      className="scenario-diagram"
      style={!animated ? { animationPlayState: 'paused' } : undefined}
      data-diagram={icon}
    >
      <Diagram accent={accent} />
    </div>
  );
}
