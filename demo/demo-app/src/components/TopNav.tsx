import { useEffect, useState } from 'react';
import { NavLink, Link } from 'react-router';
import { useApiWithFallback } from '../hooks/useApiWithFallback';
import './TopNav.css';

const NAV_LINKS = [
  { to: '/demo', label: 'DEMO' },
  { to: '/dashboard', label: 'DASHBOARD' },
  { to: '/bench', label: 'BENCH' },
  { to: '/explorer', label: 'EXPLORER' },
  { to: '/builder', label: 'BUILDER' },
  { to: '/terminal', label: 'TERMINAL' },
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
  const { get } = useApiWithFallback();
  const [online, setOnline] = useState<boolean>(false);
  const [uptime, setUptime] = useState<number | null>(null);

  useEffect(() => {
    let cancelled = false;
    const poll = async () => {
      try {
        const data = await get<HealthResponse>('/api/health');
        if (!cancelled) {
          setOnline(data.status === 'ok');
          if (data.uptime_secs != null) setUptime(data.uptime_secs);
        }
      } catch {
        if (!cancelled) setOnline(false);
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
        <b>ROKO</b>
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
        <span className={`status-pill ${online ? 'live' : 'demo'}`}>
          <span className="status-dot" />
          {online && uptime != null ? `LIVE ${fmtUptime(uptime)}` : 'DEMO'}
        </span>
      </div>
    </nav>
  );
}
