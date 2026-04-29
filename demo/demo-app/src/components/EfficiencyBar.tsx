import Mosaic, { MosaicCell } from './Mosaic';
import PhosphorNumber from './PhosphorNumber';
import './EfficiencyBar.css';

export interface EfficiencyMetric {
  label: string;
  value: number;
  format?: (n: number) => string;
  color?: 'rose' | 'bone' | 'dream' | 'success' | 'warning';
}

interface EfficiencyBarProps {
  metrics: EfficiencyMetric[];
}

export default function EfficiencyBar({ metrics }: EfficiencyBarProps) {
  const cols = Math.min(Math.max(metrics.length, 2), 6) as 2 | 3 | 4 | 5 | 6;

  return (
    <div className="efficiency-bar">
      <Mosaic columns={cols}>
        {metrics.map((m) => (
          <MosaicCell
            key={m.label}
            label={m.label}
            value={<PhosphorNumber value={m.value} format={m.format} />}
            color={m.color}
            mono
          />
        ))}
      </Mosaic>
    </div>
  );
}
