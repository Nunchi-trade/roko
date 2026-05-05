import './PhaseRail.css';

interface PhaseRailProps {
  phases: string[];
  current: number;
  failed?: number;
  className?: string;
}

function dotClass(index: number, current: number, failed?: number): string {
  if (failed != null && index === failed) return 'phase-rail__dot phase-rail__dot--failed';
  if (index < current) return 'phase-rail__dot phase-rail__dot--done';
  if (index === current) return 'phase-rail__dot phase-rail__dot--current';
  return 'phase-rail__dot phase-rail__dot--pending';
}

function labelClass(index: number, current: number, failed?: number): string {
  if (failed != null && index === failed) return 'phase-rail__label phase-rail__label--failed';
  if (index < current) return 'phase-rail__label phase-rail__label--done';
  if (index === current) return 'phase-rail__label phase-rail__label--current';
  return 'phase-rail__label phase-rail__label--pending';
}

function lineClass(index: number, current: number): string {
  const base = 'phase-rail__line';
  if (index < current) return `${base} ${base}--done`;
  if (index === current) return `${base} ${base}--current`;
  return base;
}

export function PhaseRail({ phases, current, failed, className }: PhaseRailProps) {
  const cls = className ? `phase-rail ${className}` : 'phase-rail';

  return (
    <div className={cls}>
      {phases.map((phase, i) => (
        <div key={phase} className="phase-rail__step">
          <span className={dotClass(i, current, failed)} />
          <span className={labelClass(i, current, failed)}>{phase}</span>
          {i < phases.length - 1 && (
            <span className={lineClass(i, current)} />
          )}
        </div>
      ))}
    </div>
  );
}
