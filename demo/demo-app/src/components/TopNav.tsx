import { useEffect, useState } from 'react';
import { NavLink, Link } from 'react-router';
import { useApi } from '../hooks/useApi';
import { useServerHealth } from '../hooks/useServerHealth';
import './TopNav.css';

const NAV_LINKS = [
  { to: '/demo', label: 'Demo' },
  { to: '/dashboard', label: 'Dashboard' },
  { to: '/bench', label: 'Bench' },
  { to: '/explorer', label: 'Explorer' },
  { to: '/builder', label: 'Builder' },
  { to: '/terminal', label: 'Terminal' },
  { to: '/settings', label: 'Settings' },
];

interface HealthResponse {
  status?: string;
  uptime_secs?: number;
}

function fmtUptime(secs: number): string {
  if (secs < 60) return `${Math.floor(secs)}s`;
  const h = Math.floor(secs / 3600);
  const m = Math.floor((secs % 3600) / 60);
  if (h === 0) return `${m}m`;
  return `${h}h ${m}m`;
}

export default function TopNav() {
  const { get } = useApi();
  const serverHealth = useServerHealth();
  const [uptime, setUptime] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const data = await get<HealthResponse>('/api/health');
        if (!cancelled) {
          if (data.uptime_secs != null) setUptime(data.uptime_secs);
        }
      } catch {
        if (!cancelled) setUptime(null);
      }
    };
    poll();
    const id = setInterval(poll, 5_000);
    return () => { cancelled = true; clearInterval(id); };
  }, [get]);

  return (
    <nav className="topnav">
      <Link to="/" className="brand" style={{ textDecoration: 'none' }}>
        <span className="mark" aria-hidden="true" />
        <b>nunchi</b>
      </Link>
      <div className="links">
        {NAV_LINKS.map((l) => (
          <NavLink
            key={l.to}
            to={l.to}
            className={({ isActive }) => isActive ? 'active' : ''}
          >
            {l.label}
          </NavLink>
        ))}
      </div>
      <div className="right">
        <span className={`status-pill ${serverHealth === 'connected' ? 'live' : 'demo'}`}>
          <span className="status-dot" />
          {serverHealth === 'connected' && uptime != null ? `Live ${fmtUptime(uptime)}` : 'Demo'}
        </span>
      </div>
    </nav>
  );
}
