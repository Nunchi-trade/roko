import { useEffect, useState } from 'react';
import { NavLink, Link } from 'react-router';
import { useApi } from '../hooks/useApi';
import { fmtUptime } from '../lib/format';
import { useServerHealth } from '../hooks/useServerHealth';
import './TopNav.css';

const NAV_LINKS = [
  { to: '/demo', label: 'DEMO' },
  { to: '/dashboard', label: 'DASH' },
  { to: '/bench', label: 'BENCH' },
  { to: '/explorer', label: 'EXPLORE' },
  { to: '/builder', label: 'BUILD' },
  { to: '/terminal', label: 'TERM' },
  { to: '/settings', label: 'CONFIG' },
];

interface HealthResponse {
  status?: string;
  uptime_secs?: number;
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

  const isLive = serverHealth === 'connected' && uptime != null;

  return (
    <nav className="topnav">
      <Link to="/" className="brand" style={{ textDecoration: 'none' }}>
        <span className="mark" aria-hidden="true" />
        <span className="brand-text">{'\u2308'} NUNCHI {'\u230B'}</span>
      </Link>

      <span className="nav-sep" aria-hidden="true">{'\u2502'}</span>

      <div className="links">
        {NAV_LINKS.map((l) => (
          <NavLink
            key={l.to}
            to={l.to}
            className={({ isActive }) => `nav-link${isActive ? ' active' : ''}`}
          >
            {({ isActive }) => (
              <span className="nav-link-inner">
                {isActive ? `\u25B8 ${l.label} \u25C2` : l.label}
              </span>
            )}
          </NavLink>
        ))}
      </div>

      <div className="right">
        <span className={`status-pill ${isLive ? 'live' : 'demo'}`}>
          <span className="status-char">{isLive ? '\u25CF' : '\u25CB'}</span>
          {isLive ? `LIVE ${fmtUptime(uptime)}` : 'SEED'}
        </span>
      </div>
    </nav>
  );
}
