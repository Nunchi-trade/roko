import './FlatIcon.css';

export type FlatIconName =
  | 'activity'
  | 'agent'
  | 'bench'
  | 'builder'
  | 'check'
  | 'clock'
  | 'cost'
  | 'dashboard'
  | 'database'
  | 'demo'
  | 'duration'
  | 'event'
  | 'explorer'
  | 'gate'
  | 'hash'
  | 'model'
  | 'provider'
  | 'refresh'
  | 'route'
  | 'settings'
  | 'spark'
  | 'status'
  | 'task'
  | 'terminal'
  | 'workflow';

interface FlatIconProps {
  name: FlatIconName;
  size?: number;
  tone?: 'rose' | 'bone' | 'dream' | 'success' | 'warning' | 'muted';
  className?: string;
  title?: string;
}

const PATHS: Record<FlatIconName, string[]> = {
  activity: ['M4 13h3l2-8 4 14 2-6h5'],
  agent: ['M12 4a4 4 0 1 1 0 8 4 4 0 0 1 0-8Z', 'M4.5 21a7.5 7.5 0 0 1 15 0'],
  bench: ['M5 5h14v5H5Z', 'M7 10v9', 'M17 10v9', 'M4 19h16'],
  builder: ['M4 17 17 4l3 3L7 20H4Z', 'M14 7l3 3'],
  check: ['M5 12l4 4L19 6'],
  clock: ['M12 3a9 9 0 1 1 0 18 9 9 0 0 1 0-18Z', 'M12 7v5l3 2'],
  cost: ['M12 3v18', 'M17 7.5A4 4 0 0 0 12 6c-3 0-5 1.2-5 3s2 2.5 5 3 5 1.2 5 3-2 3-5 3a6 6 0 0 1-5.5-2.2'],
  dashboard: ['M4 13h6V4H4Z', 'M14 20h6V4h-6Z', 'M4 20h6v-4H4Z'],
  database: ['M5 7c0-2 14-2 14 0s-14 2-14 0Z', 'M5 7v5c0 2 14 2 14 0V7', 'M5 12v5c0 2 14 2 14 0v-5'],
  demo: ['M5 6h14v10H5Z', 'M9 20h6', 'M12 16v4'],
  duration: ['M12 4a8 8 0 1 1-8 8', 'M12 7v5l4 2', 'M4 4v5h5'],
  event: ['M5 5h14v14H5Z', 'M8 9h8', 'M8 13h5'],
  explorer: ['M11 5a6 6 0 1 1 0 12 6 6 0 0 1 0-12Z', 'M16 16l4 4'],
  gate: ['M5 20V8l7-4 7 4v12', 'M9 20v-7h6v7'],
  hash: ['M8 4 6 20', 'M18 4l-2 16', 'M4 9h16', 'M3 15h16'],
  model: ['M12 4l8 4-8 4-8-4Z', 'M4 12l8 4 8-4', 'M4 16l8 4 8-4'],
  provider: ['M12 4a8 8 0 0 1 8 8', 'M12 4a8 8 0 0 0-8 8', 'M4 12h16', 'M12 20a8 8 0 0 0 8-8', 'M12 20a8 8 0 0 1-8-8'],
  refresh: ['M5 8a7 7 0 0 1 12-3l2 2', 'M19 5v5h-5', 'M19 16a7 7 0 0 1-12 3l-2-2', 'M5 19v-5h5'],
  route: ['M6 5a3 3 0 1 1 0 6 3 3 0 0 1 0-6Z', 'M18 13a3 3 0 1 1 0 6 3 3 0 0 1 0-6Z', 'M9 8h4a4 4 0 0 1 4 4v1'],
  settings: ['M12 8a4 4 0 1 1 0 8 4 4 0 0 1 0-8Z', 'M12 3v3', 'M12 18v3', 'M3 12h3', 'M18 12h3', 'M5.6 5.6l2.1 2.1', 'M16.3 16.3l2.1 2.1', 'M18.4 5.6l-2.1 2.1', 'M7.7 16.3l-2.1 2.1'],
  spark: ['M4 15l4-4 3 3 5-7 4 5'],
  status: ['M12 4a8 8 0 1 1 0 16 8 8 0 0 1 0-16Z', 'M9 12l2 2 4-5'],
  task: ['M6 4h12v16H6Z', 'M9 8h6', 'M9 12h6', 'M9 16h4'],
  terminal: ['M4 5h16v14H4Z', 'M7 9l3 3-3 3', 'M12 15h5'],
  workflow: ['M5 7h5v5H5Z', 'M14 12h5v5h-5Z', 'M10 9h2a4 4 0 0 1 4 4'],
};

export function inferIcon(label: string): FlatIconName {
  const text = label.toLowerCase();
  if (text.includes('demo')) return 'demo';
  if (text.includes('dashboard')) return 'dashboard';
  if (text.includes('bench')) return 'bench';
  if (text.includes('explor')) return 'explorer';
  if (text.includes('build')) return 'builder';
  if (text.includes('terminal')) return 'terminal';
  if (text.includes('setting') || text.includes('config')) return 'settings';
  if (text.includes('episode') || text.includes('entries') || text.includes('knowledge')) return 'database';
  if (text.includes('cost') || text.includes('$')) return 'cost';
  if (text.includes('agent') || text.includes('fleet')) return 'agent';
  if (text.includes('gate') || text.includes('pass') || text.includes('integrity')) return 'gate';
  if (text.includes('duration') || text.includes('time') || text.includes('uptime')) return 'duration';
  if (text.includes('activity') || text.includes('density') || text.includes('event')) return 'activity';
  if (text.includes('provider') || text.includes('health')) return 'provider';
  if (text.includes('model') || text.includes('routing')) return 'model';
  if (text.includes('hash') || text.includes('trail')) return 'hash';
  if (text.includes('workflow') || text.includes('pipeline')) return 'workflow';
  if (text.includes('task') || text.includes('plan')) return 'task';
  if (text.includes('status')) return 'status';
  return 'spark';
}

export default function FlatIcon({ name, size = 18, tone = 'rose', className, title }: FlatIconProps) {
  return (
    <span
      className={`flat-icon flat-icon--${tone}${className ? ` ${className}` : ''}`}
      style={{ '--icon-size': `${size}px` } as React.CSSProperties}
      aria-hidden={title ? undefined : true}
      role={title ? 'img' : undefined}
      aria-label={title}
    >
      <svg viewBox="0 0 24 24" focusable="false">
        {PATHS[name].map((d) => (
          <path key={d} d={d} />
        ))}
      </svg>
    </span>
  );
}
