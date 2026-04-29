import { useState, useCallback, useMemo } from 'react';
import './CellGraph.css';

interface GraphNode {
  id: string;
  label: string;
  x: number;
  y: number;
  status?: string;
}

interface GraphEdge {
  from: string;
  to: string;
  label?: string;
}

interface CellGraphProps {
  nodes: GraphNode[];
  edges: GraphEdge[];
  width?: number;
  height?: number;
  onNodeClick?: (id: string) => void;
}

const NODE_W = 80;
const NODE_H = 28;

function statusClass(status?: string): string {
  if (!status) return 'idle';
  const s = status.toLowerCase();
  if (s === 'active' || s === 'running') return 'active';
  if (s === 'success' || s === 'completed' || s === 'done' || s === 'pass') return 'success';
  if (s === 'error' || s === 'failed' || s === 'fail') return 'error';
  if (s === 'blocked' || s === 'pending') return 'blocked';
  return 'idle';
}

function truncate(text: string, max: number): string {
  return text.length > max ? text.slice(0, max - 1) + '\u2026' : text;
}

export function CellGraph({
  nodes,
  edges,
  width = 400,
  height = 300,
  onNodeClick,
}: CellGraphProps) {
  const [hoveredId, setHoveredId] = useState<string | null>(null);

  const nodeMap = useMemo(() => {
    const m = new Map<string, GraphNode>();
    for (const n of nodes) m.set(n.id, n);
    return m;
  }, [nodes]);

  const connectedIds = useMemo(() => {
    if (!hoveredId) return new Set<string>();
    const ids = new Set<string>([hoveredId]);
    for (const e of edges) {
      if (e.from === hoveredId) ids.add(e.to);
      if (e.to === hoveredId) ids.add(e.from);
    }
    return ids;
  }, [hoveredId, edges]);

  const handleNodeEnter = useCallback((id: string) => setHoveredId(id), []);
  const handleNodeLeave = useCallback(() => setHoveredId(null), []);

  return (
    <div className="cell-graph" style={{ width, height }}>
      <svg
        className="cell-graph__svg"
        viewBox={`0 0 ${width} ${height}`}
        preserveAspectRatio="xMidYMid meet"
      >
        {/* Edges */}
        {edges.map((edge) => {
          const fromNode = nodeMap.get(edge.from);
          const toNode = nodeMap.get(edge.to);
          if (!fromNode || !toNode) return null;

          const highlighted =
            hoveredId !== null &&
            (edge.from === hoveredId || edge.to === hoveredId);

          const mx = (fromNode.x + toNode.x) / 2;
          const my = (fromNode.y + toNode.y) / 2;

          return (
            <g key={`${edge.from}-${edge.to}`}>
              <line
                className={`cell-graph__edge${highlighted ? ' cell-graph__edge--highlighted' : ''}`}
                x1={fromNode.x}
                y1={fromNode.y}
                x2={toNode.x}
                y2={toNode.y}
              />
              {edge.label && (
                <text className="cell-graph__edge-label" x={mx} y={my - 6}>
                  {edge.label}
                </text>
              )}
            </g>
          );
        })}

        {/* Nodes */}
        {nodes.map((node) => {
          const highlighted = hoveredId !== null && connectedIds.has(node.id);
          const sc = statusClass(node.status);

          return (
            <g
              key={node.id}
              className="cell-graph__node"
              transform={`translate(${node.x - NODE_W / 2}, ${node.y - NODE_H / 2})`}
              onClick={() => onNodeClick?.(node.id)}
              onMouseEnter={() => handleNodeEnter(node.id)}
              onMouseLeave={handleNodeLeave}
            >
              <rect
                className={[
                  'cell-graph__node-rect',
                  `cell-graph__node-rect--${sc}`,
                  highlighted && 'cell-graph__node-rect--highlighted',
                ]
                  .filter(Boolean)
                  .join(' ')}
                width={NODE_W}
                height={NODE_H}
              />
              <text
                className="cell-graph__node-label"
                x={NODE_W / 2}
                y={NODE_H / 2}
              >
                {truncate(node.label, 10)}
              </text>
            </g>
          );
        })}
      </svg>
    </div>
  );
}

export type { GraphNode, GraphEdge, CellGraphProps };
