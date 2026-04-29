import type { CSSProperties } from 'react';
import { NavLink, Outlet } from 'react-router';
import FlatIcon, { type FlatIconName } from '../../components/FlatIcon';

const VIEWS = [
  { to: '/dashboard', label: 'Cost', icon: 'cost', end: true },
  { to: '/dashboard/fleet', label: 'Fleet', icon: 'agent', end: false },
  { to: '/dashboard/knowledge', label: 'Knowledge', icon: 'database', end: false },
  { to: '/dashboard/entries', label: 'Entries', icon: 'event', end: false },
  { to: '/dashboard/routing', label: 'Routing', icon: 'route', end: false },
  { to: '/dashboard/integrity', label: 'Integrity', icon: 'hash', end: false },
  { to: '/dashboard/dreams', label: 'Dreams', icon: 'spark', end: false },
] satisfies Array<{ to: string; label: string; icon: FlatIconName; end: boolean }>;

const shellStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  flex: 1,
  minHeight: 0,
  overflow: 'hidden',
};

const navStyle: CSSProperties = {
  display: 'flex',
  gap: 4,
  padding: 'var(--sp-1) var(--sp-6)',
  borderBottom: '1px solid var(--glass-2-border)',
  background: 'rgba(8, 8, 12, 0.7)',
  backdropFilter: 'blur(8px)',
  overflowX: 'auto',
  flexShrink: 0,
};

const linkStyle: CSSProperties = {
  border: '1px solid transparent',
  borderRadius: 'var(--radius-md)',
  color: 'var(--text-primary)',
  display: 'inline-flex',
  alignItems: 'center',
  gap: 8,
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: 'var(--text-sm)',
  letterSpacing: '.04em',
  padding: 'var(--sp-1) var(--sp-4)',
  textDecoration: 'none',
  whiteSpace: 'nowrap',
  transition: 'all .2s ease',
};

const activeLinkStyle: CSSProperties = {
  color: 'var(--rose-bright)',
  background: 'var(--rose-deep)',
  borderColor: 'var(--rose-dim)',
};

const bodyStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
  padding: 'var(--sp-3) var(--sp-6) var(--sp-5)',
  overflowY: 'auto',
};

export default function DashboardLayout() {
  return (
    <div style={shellStyle}>
      <nav style={navStyle} aria-label="Dashboard sections">
        {VIEWS.map((view) => (
          <NavLink
            key={view.to}
            to={view.to}
            end={view.end}
            style={({ isActive }) => ({
              ...linkStyle,
              ...(isActive ? activeLinkStyle : null),
            })}
          >
            <FlatIcon name={view.icon} size={13} tone="muted" />
            {view.label}
          </NavLink>
        ))}
      </nav>
      <div style={bodyStyle}>
        <Outlet />
      </div>
    </div>
  );
}
