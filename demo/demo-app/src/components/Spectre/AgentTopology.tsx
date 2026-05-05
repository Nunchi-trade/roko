import { useState, useMemo, useRef, useCallback, useEffect } from 'react';
import { type AgentIdentity, ROLE_PALETTES } from './AgentIdentity';
import KnowledgeTransfer from './KnowledgeTransfer';
import './AgentTopology.css';

/* ── Public types ─────────────────────────────────────── */

export interface TopologyNode {
  id: string;
  identity: AgentIdentity;
  status: 'active' | 'idle' | 'error' | 'completed';
  x?: number;
  y?: number;
}

export interface TopologyEdge {
  source: string;
  target: string;
  type: 'dependency' | 'communication' | 'knowledge';
  active?: boolean;
}

export interface AgentTopologyProps {
  nodes: TopologyNode[];
  edges: TopologyEdge[];
  width?: number;
  height?: number;
}

/* ── Layout helper ────────────────────────────────────── */

interface LayoutNode extends TopologyNode {
  cx: number;
  cy: number;
}

/**
 * Simple hierarchical layout:
 *   - orchestrator/lead at the top center
 *   - remaining nodes in a semicircle below
 * Falls back to circular layout if no orchestrator is found.
 */
function layoutNodes(
  nodes: TopologyNode[],
  w: number,
  h: number,
): LayoutNode[] {
  if (nodes.length === 0) return [];

  const pad = 48;
  const usableW = w - pad * 2;
  const usableH = h - pad * 2;

  // Use explicit position if provided
  const hasExplicit = nodes.every((n) => n.x != null && n.y != null);
  if (hasExplicit) {
    return nodes.map((n) => ({ ...n, cx: n.x!, cy: n.y! }));
  }

  // Find orchestrator / lead
  const orchIdx = nodes.findIndex(
    (n) =>
      n.identity.archetype === 'orchestrator' ||
      n.identity.role === 'lead',
  );

  if (orchIdx >= 0 && nodes.length > 1) {
    // Hierarchical: orchestrator at top, rest in an arc below
    const result: LayoutNode[] = [];
    result.push({
      ...nodes[orchIdx],
      cx: w / 2,
      cy: pad + 20,
    });

    const rest = nodes.filter((_, i) => i !== orchIdx);
    const cols = Math.min(rest.length, 5);
    const rows = Math.ceil(rest.length / cols);
    rest.forEach((n, i) => {
      const row = Math.floor(i / cols);
      const col = i % cols;
      const totalInRow = Math.min(cols, rest.length - row * cols);
      const xSpacing = usableW / (totalInRow + 1);
      result.push({
        ...n,
        cx: pad + xSpacing * (col + 1),
        cy: pad + 70 + row * (usableH / (rows + 1)),
      });
    });

    return result;
  }

  // Circular fallback
  return nodes.map((n, i) => {
    const angle = (i / nodes.length) * Math.PI * 2 - Math.PI / 2;
    const rx = usableW * 0.38;
    const ry = usableH * 0.38;
    return {
      ...n,
      cx: w / 2 + Math.cos(angle) * rx,
      cy: h / 2 + Math.sin(angle) * ry,
    };
  });
}

/* ── Node radius by role ──────────────────────────────── */

function nodeRadius(identity: AgentIdentity): number {
  if (identity.archetype === 'orchestrator' || identity.role === 'lead') return 22;
  return 16;
}

function nodeColor(identity: AgentIdentity): string {
  const palette = ROLE_PALETTES[identity.role];
  return palette ? palette[0] : '#9A8A98';
}

/* ── Component ────────────────────────────────────────── */

export default function AgentTopology({
  nodes,
  edges,
  width = 600,
  height = 300,
}: AgentTopologyProps) {
  const containerRef = useRef<HTMLDivElement>(null);
  const [hovered, setHovered] = useState<string | null>(null);
  const [tooltipPos, setTooltipPos] = useState<{ x: number; y: number } | null>(null);

  const layout = useMemo(() => layoutNodes(nodes, width, height), [nodes, width, height]);
  const nodeMap = useMemo(() => new Map(layout.map((n) => [n.id, n])), [layout]);

  const handleMouseEnter = useCallback(
    (id: string, cx: number, cy: number) => {
      setHovered(id);
      setTooltipPos({ x: cx, y: cy });
    },
    [],
  );

  const handleMouseLeave = useCallback(() => {
    setHovered(null);
    setTooltipPos(null);
  }, []);

  // Resolve tooltip screen position from SVG coords
  const [tooltipScreen, setTooltipScreen] = useState<{ left: number; top: number } | null>(null);
  const svgRef = useRef<SVGSVGElement>(null);

  useEffect(() => {
    if (!tooltipPos || !svgRef.current || !containerRef.current) {
      setTooltipScreen(null);
      return;
    }
    const containerRect = containerRef.current.getBoundingClientRect();
    const svgRect = svgRef.current.getBoundingClientRect();
    const scaleX = svgRect.width / width;
    const scaleY = svgRect.height / height;
    setTooltipScreen({
      left: (tooltipPos.x * scaleX) + (svgRect.left - containerRect.left),
      top: (tooltipPos.y * scaleY) + (svgRect.top - containerRect.top) - 8,
    });
  }, [tooltipPos, width, height]);

  if (nodes.length === 0) {
    return (
      <div className="agent-topology" style={{ height }}>
        <div className="at-empty">no agents in topology</div>
      </div>
    );
  }

  const hoveredNode = hovered ? nodeMap.get(hovered) : null;

  return (
    <div ref={containerRef} className="agent-topology" style={{ height }}>
      <svg
        ref={svgRef}
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="xMidYMid meet"
      >
        <defs>
          <radialGradient id="at-bg-grad" cx="50%" cy="40%" r="70%">
            <stop offset="0%" stopColor="color-mix(in srgb, var(--rose) 6%, transparent)" />
            <stop offset="100%" stopColor="rgba(9, 11, 15, 0)" />
          </radialGradient>
          {/* Edge path definitions for knowledge transfer particles */}
          {edges.map((edge, i) => {
            const src = nodeMap.get(edge.source);
            const tgt = nodeMap.get(edge.target);
            if (!src || !tgt) return null;
            return (
              <path
                key={`path-${i}`}
                id={`at-edge-path-${i}`}
                d={`M ${src.cx} ${src.cy} L ${tgt.cx} ${tgt.cy}`}
                fill="none"
                stroke="none"
              />
            );
          })}
        </defs>

        {/* Background */}
        <rect width={width} height={height} fill="url(#at-bg-grad)" />

        {/* Edges */}
        {edges.map((edge, i) => {
          const src = nodeMap.get(edge.source);
          const tgt = nodeMap.get(edge.target);
          if (!src || !tgt) return null;

          const edgeClass = [
            'at-edge',
            `at-edge--${edge.type}`,
            edge.active && 'at-edge--active',
          ]
            .filter(Boolean)
            .join(' ');

          return (
            <line
              key={`edge-${i}`}
              x1={src.cx}
              y1={src.cy}
              x2={tgt.cx}
              y2={tgt.cy}
              className={edgeClass}
            />
          );
        })}

        {/* Knowledge transfer particles on knowledge edges */}
        {edges.map((edge, i) => {
          if (edge.type !== 'knowledge' || !edge.active) return null;
          const src = nodeMap.get(edge.source);
          const tgt = nodeMap.get(edge.target);
          if (!src || !tgt) return null;
          return (
            <KnowledgeTransfer
              key={`kt-${i}`}
              pathId={`at-edge-path-${i}`}
              color="rgba(148, 148, 180, 0.8)"
              count={3}
              duration={2.5}
            />
          );
        })}

        {/* Nodes */}
        {layout.map((node) => {
          const r = nodeRadius(node.identity);
          const color = nodeColor(node.identity);
          const isHovered = hovered === node.id;

          return (
            <g
              key={node.id}
              className={`at-node at-node--${node.status}`}
              onMouseEnter={() => handleMouseEnter(node.id, node.cx, node.cy)}
              onMouseLeave={handleMouseLeave}
              style={{ opacity: hovered && !isHovered ? 0.5 : 1 }}
            >
              {/* Pulse ring for active agents */}
              <circle
                className="at-node-pulse"
                cx={node.cx}
                cy={node.cy}
                r={r + 6}
                stroke={color}
              />

              {/* Halo glow */}
              <circle
                cx={node.cx}
                cy={node.cy}
                r={r + 8}
                fill="none"
                stroke={color}
                strokeWidth={0.5}
                opacity={isHovered ? 0.4 : 0.15}
              />

              {/* Main circle fill */}
              <circle
                className="at-node-fill"
                cx={node.cx}
                cy={node.cy}
                r={r}
                fill={color}
                opacity={0.18}
              />

              {/* Ring */}
              <circle
                className="at-node-ring"
                cx={node.cx}
                cy={node.cy}
                r={r}
                stroke={color}
              />

              {/* Status indicator */}
              <circle
                className={`at-status-dot at-status-dot--${node.status}`}
                cx={node.cx + r * 0.7}
                cy={node.cy - r * 0.7}
              />

              {/* Agent initial inside circle */}
              <text
                x={node.cx}
                y={node.cy + 1}
                textAnchor="middle"
                dominantBaseline="central"
                fill={color}
                fontFamily="var(--mono)"
                fontSize={r * 0.7}
                fontWeight={600}
                opacity={0.9}
              >
                {node.identity.name.charAt(0).toUpperCase()}
              </text>

              {/* Role label above */}
              <text
                className="at-node-role"
                x={node.cx}
                y={node.cy - r - 12}
              >
                {node.identity.role}
              </text>

              {/* Name label below */}
              <text
                className="at-node-label"
                x={node.cx}
                y={node.cy + r + 14}
              >
                {node.identity.name}
              </text>
            </g>
          );
        })}
      </svg>

      {/* Tooltip */}
      {hoveredNode && tooltipScreen && (
        <div
          className="at-tooltip"
          style={{
            left: tooltipScreen.left + 20,
            top: tooltipScreen.top - 40,
          }}
        >
          <span className="at-tooltip__name">{hoveredNode.identity.name}</span>
          <div className="at-tooltip__row">
            <span className="at-tooltip__label">role</span>
            <span className="at-tooltip__value">{hoveredNode.identity.role}</span>
          </div>
          <div className="at-tooltip__row">
            <span className="at-tooltip__label">archetype</span>
            <span className="at-tooltip__value">{hoveredNode.identity.archetype}</span>
          </div>
          <div className="at-tooltip__row">
            <span className="at-tooltip__label">status</span>
            <span className="at-tooltip__value">{hoveredNode.status}</span>
          </div>
        </div>
      )}
    </div>
  );
}
