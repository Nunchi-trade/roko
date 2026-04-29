import { agentColor } from '../agent/utils';
import { shortModel } from '../../lib/format';
import './TraceAnnotation.css';

interface TraceAnnotationProps {
  /** Agent identity */
  agentName?: string;
  /** Override auto-derived agent color */
  agentColor?: string;

  /** Inference info */
  tier?: 'T0' | 'T1' | 'T2';
  model?: string;
  provider?: string;

  /** Cybernetic state: 0-1, drives background intensity of the whole strip */
  confidence?: number;

  /** Cost in dollars */
  cost?: number;
  /** Total tokens (combined in/out) */
  tokens?: number;
  /** Latency in milliseconds */
  latencyMs?: number;

  /** Minimal vs full annotation */
  compact?: boolean;
  className?: string;
}

/* ── Formatting helpers ── */

function fmtTokens(n: number): string {
  if (n >= 1000) return `${(n / 1000).toFixed(1)}k`;
  return String(n);
}

function fmtCost(c: number): string {
  if (c < 0.001) return `$${c.toFixed(4)}`;
  if (c < 0.01) return `$${c.toFixed(3)}`;
  return `$${c.toFixed(2)}`;
}

function costClass(c: number): string {
  if (c < 0.01) return 'trace-ann__cost--low';
  if (c <= 0.10) return 'trace-ann__cost--mid';
  return 'trace-ann__cost--high';
}

function fmtLatency(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.round(ms)}ms`;
}

function fmtConfidence(c: number): string {
  return `${Math.round(c * 100)}%`;
}

/** Confidence band for text color class. */
function confBand(c: number): string {
  if (c < 0.3) return 'trace-ann--conf-ghost';
  if (c < 0.6) return 'trace-ann--conf-dim';
  if (c < 0.8) return 'trace-ann--conf-normal';
  return 'trace-ann--conf-strong';
}

/** Confidence-driven inline styles for the strip background. */
function confStyle(c: number): React.CSSProperties {
  const style: React.CSSProperties = {
    backgroundColor: `rgba(184,122,148, ${c * 0.08})`,
  };
  if (c > 0.8) {
    style.boxShadow = `0 0 8px rgba(184,122,148, ${(c - 0.8) * 0.5})`;
  }
  return style;
}

export default function TraceAnnotation({
  agentName,
  agentColor: agentColorOverride,
  tier,
  model,
  provider: _provider,
  confidence,
  cost,
  tokens,
  latencyMs,
  compact = false,
  className,
}: TraceAnnotationProps) {
  const conf = confidence ?? 0;
  const color = agentColorOverride ?? (agentName ? agentColor(agentName) : undefined);

  const classes = [
    'trace-ann',
    confBand(conf),
    className,
  ].filter(Boolean).join(' ');

  return (
    <span className={classes} style={confStyle(conf)}>
      {/* Agent badge */}
      {agentName && (
        <span className="trace-ann__agent">
          <span
            className="trace-ann__agent-dot"
            style={{ backgroundColor: color }}
          />
          <span className="trace-ann__agent-name">{agentName}</span>
        </span>
      )}

      {/* Tier + Model */}
      {tier && (
        <span className="trace-ann__tier-model">
          <span className={`trace-ann__tier trace-ann__tier--${tier.toLowerCase()}`}>
            {tier}
          </span>
          {model && (
            <span className="trace-ann__model">{shortModel(model)}</span>
          )}
        </span>
      )}

      {/* Everything below is hidden in compact mode */}
      {!compact && confidence != null && (
        <span className="trace-ann__conf">{fmtConfidence(confidence)}</span>
      )}

      {!compact && tokens != null && (
        <span className="trace-ann__tokens">{fmtTokens(tokens)} tok</span>
      )}

      {!compact && cost != null && (
        <span className={`trace-ann__cost ${costClass(cost)}`}>
          {fmtCost(cost)}
        </span>
      )}

      {!compact && latencyMs != null && (
        <span className="trace-ann__latency">{fmtLatency(latencyMs)}</span>
      )}
    </span>
  );
}
