import type { CSSProperties } from 'react';
import { NavLink, Outlet } from 'react-router';

const VIEWS = [
  { to: '/dashboard', label: 'Cost', end: true },
  { to: '/dashboard/fleet', label: 'Fleet', end: false },
  { to: '/dashboard/knowledge', label: 'Knowledge', end: false },
  { to: '/dashboard/entries', label: 'Entries', end: false },
  { to: '/dashboard/routing', label: 'Routing', end: false },
  { to: '/dashboard/integrity', label: 'Integrity', end: false },
  { to: '/dashboard/dreams', label: 'Dreams', end: false },
];

const shellStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  minHeight: 'calc(100vh - 48px)',
  padding: 0,
};

const navStyle: CSSProperties = {
  display: 'flex',
  gap: 4,
  padding: '6px 24px',
  borderBottom: '1px solid var(--glass-2-border)',
  background: 'rgba(8, 8, 12, 0.6)',
  backdropFilter: 'blur(8px)',
  overflowX: 'auto',
  position: 'sticky',
  top: 48,
  zIndex: 100,
};

const linkStyle: CSSProperties = {
  border: '1px solid transparent',
  borderRadius: 6,
  color: 'var(--text-dim)',
  fontFamily: 'var(--mono, var(--font-mono))',
  fontSize: '0.68rem',
  letterSpacing: '.04em',
  padding: '6px 16px',
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
  padding: '12px 24px 20px',
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
