import type { CSSProperties } from 'react';
import { NavLink, Outlet } from 'react-router';

const VIEWS = [
  { to: '/dashboard', label: 'Cost', end: true },
  { to: '/dashboard/fleet', label: 'Fleet', end: false },
  { to: '/dashboard/knowledge', label: 'Knowledge', end: false },
  { to: '/dashboard/entries', label: 'Entries', end: false },
  { to: '/dashboard/routing', label: 'Routing', end: false },
  { to: '/dashboard/chain', label: 'Chain', end: false },
  { to: '/dashboard/dreams', label: 'Dreams', end: false },
];

const shellStyle: CSSProperties = {
  display: 'flex',
  flexDirection: 'column',
  minHeight: '100%',
};

const navStyle: CSSProperties = {
  display: 'flex',
  gap: 2,
  padding: '8px 40px',
  borderBottom: '1px solid var(--glass-2-border)',
  background: 'var(--raised)',
  overflowX: 'auto',
};

const linkStyle: CSSProperties = {
  border: '1px solid transparent',
  borderRadius: 6,
  color: 'var(--text-dim)',
  fontFamily: 'var(--font-sans)',
  fontSize: '0.72rem',
  padding: '6px 14px',
  textDecoration: 'none',
  whiteSpace: 'nowrap',
};

const activeLinkStyle: CSSProperties = {
  color: 'var(--rose-bright)',
  background: 'var(--rose-deep)',
  borderColor: 'var(--rose-dim)',
};

const bodyStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
};

export default function DashboardLayout() {
  return (
    <section style={shellStyle}>
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
    </section>
  );
}
