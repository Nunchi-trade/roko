import { useEffect, useState } from 'react';
import { NavLink, Link } from 'react-router';
import { useApi } from '../hooks/useApi';
import { fmtUptime } from '../lib/format';
import { useServerHealth } from '../hooks/useServerHealth';
import FlatIcon, { type FlatIconName } from './FlatIcon';
import './TopNav.css';

const NAV_LINKS = [
  { to: '/demo', label: 'Demo', icon: 'demo' },
  { to: '/dashboard', label: 'Dashboard', icon: 'dashboard' },
  { to: '/bench', label: 'Bench', icon: 'bench' },
  { to: '/explorer', label: 'Explorer', icon: 'explorer' },
  { to: '/builder', label: 'Builder', icon: 'builder' },
  { to: '/terminal', label: 'Terminal', icon: 'terminal' },
  { to: '/settings', label: 'Settings', icon: 'settings' },
] satisfies Array<{ to: string; label: string; icon: FlatIconName }>;

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
            <FlatIcon name={l.icon} size={13} tone="muted" />
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
