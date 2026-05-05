import { shortModel } from '../../lib/format';
import './InferenceTag.css';

interface InferenceTagProps {
  tier: 'T0' | 'T1' | 'T2';
  model: string;
  provider?: string;
  inputTokens?: number;
  outputTokens?: number;
  cost?: number;
  latencyMs?: number;
  compact?: boolean;
  className?: string;
}

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
  if (c < 0.01) return 'inference-tag__cost--low';
  if (c <= 0.10) return 'inference-tag__cost--mid';
  return 'inference-tag__cost--high';
}

function fmtLatency(ms: number): string {
  if (ms >= 1000) return `${(ms / 1000).toFixed(1)}s`;
  return `${Math.round(ms)}ms`;
}

export default function InferenceTag({
  tier,
  model,
  provider: _provider,
  inputTokens,
  outputTokens,
  cost,
  latencyMs,
  compact = false,
  className,
}: InferenceTagProps) {
  const tierKey = tier.toLowerCase() as 't0' | 't1' | 't2';
  const displayModel = shortModel(model);

  return (
    <span className={`inference-tag${className ? ` ${className}` : ''}`}>
      <span className={`inference-tag__tier inference-tag__tier--${tierKey}`} />
      <span className={`inference-tag__tier-label inference-tag__tier-label--${tierKey}`}>
        {tier}
      </span>
      <span className="inference-tag__sep">{'\u25b8'}</span>
      <span className="inference-tag__model">{displayModel}</span>

      {!compact && inputTokens != null && (
        <span className="inference-tag__tokens">
          <span className="inference-tag__tok-in">{'\u2192'}{fmtTokens(inputTokens)}</span>
        </span>
      )}
      {!compact && outputTokens != null && (
        <span className="inference-tag__tokens">
          <span className="inference-tag__tok-out">{'\u2190'}{fmtTokens(outputTokens)}</span>
        </span>
      )}
      {!compact && cost != null && (
        <span className={`inference-tag__cost ${costClass(cost)}`}>
          {fmtCost(cost)}
        </span>
      )}
      {!compact && latencyMs != null && (
        <span className="inference-tag__latency">{fmtLatency(latencyMs)}</span>
      )}
    </span>
  );
}
