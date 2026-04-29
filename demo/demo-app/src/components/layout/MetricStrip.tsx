import { AnimatedNumber } from '../motion/AnimatedNumber';
import './MetricStrip.css';

interface Metric {
  label: string;
  value: number;
  format?: (n: number) => string;
  suffix?: string;
}

interface MetricStripProps {
  metrics: Metric[];
  className?: string;
}

export function MetricStrip({ metrics, className }: MetricStripProps) {
  const cls = className ? `metric-strip ${className}` : 'metric-strip';

  return (
    <div className={cls}>
      {metrics.map((m) => (
        <div key={m.label} className="metric-strip__item">
          <span className="metric-strip__label">{m.label}</span>
          <span className="metric-strip__value">
            <AnimatedNumber value={m.value} format={m.format} />
            {m.suffix && (
              <span className="metric-strip__suffix">{m.suffix}</span>
            )}
          </span>
        </div>
      ))}
    </div>
  );
}
