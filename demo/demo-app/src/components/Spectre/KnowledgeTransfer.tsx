import './KnowledgeTransfer.css';

interface KnowledgeTransferProps {
  /** SVG path id or d-string for the edge */
  pathId: string;
  /** Color of the particles */
  color?: string;
  /** Number of particles along the edge */
  count?: number;
  /** Duration of one particle traversal in seconds */
  duration?: number;
}

/**
 * Animated particles that travel along an SVG edge path,
 * visualizing knowledge transfer between agents.
 * Uses SVG <animateMotion> for broad browser support.
 */
export default function KnowledgeTransfer({
  pathId,
  color = 'rgba(148, 148, 180, 0.8)',
  count = 3,
  duration = 2.5,
}: KnowledgeTransferProps) {
  const particles = Array.from({ length: count }, (_, i) => {
    const delay = (i / count) * duration;
    return (
      <g key={i}>
        {/* Glow */}
        <circle className="kt-particle-glow" fill={color}>
          <animateMotion
            dur={`${duration}s`}
            begin={`${delay}s`}
            repeatCount="indefinite"
          >
            <mpath href={`#${pathId}`} />
          </animateMotion>
        </circle>
        {/* Core dot */}
        <circle className="kt-particle" r="2" fill={color}>
          <animateMotion
            dur={`${duration}s`}
            begin={`${delay}s`}
            repeatCount="indefinite"
          >
            <mpath href={`#${pathId}`} />
          </animateMotion>
        </circle>
      </g>
    );
  });

  return <g className="kt-particles">{particles}</g>;
}
