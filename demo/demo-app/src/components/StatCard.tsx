import './StatCard.css';

interface StatCardProps {
  label: string;
  value: string | number;
  sub?: string;
  color?: 'rose' | 'bone' | 'sage' | 'fail' | 'warn';
}

export default function StatCard({ label, value, sub, color = 'bone' }: StatCardProps) {
  return (
    <div className={`stat-card stat-${color}`}>
      <div className="stat-value">{value}</div>
      <div className="stat-label">{label}</div>
      {sub && <div className="stat-sub">{sub}</div>}
    </div>
  );
}
