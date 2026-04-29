import React, { type CSSProperties } from 'react';
import { NavLink, Outlet, useLocation } from 'react-router';
import FlatIcon, { type FlatIconName } from '../../components/FlatIcon';
import RevealWhen from '../../components/RevealWhen';

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
};

const navStyle: CSSProperties = {
  display: 'flex',
  gap: 4,
  padding: 'var(--sp-1) var(--sp-4)',
  borderBottom: '1px solid var(--glass-2-border)',
  background: 'var(--bg-deeper)',
  backdropFilter: 'blur(12px) saturate(180%)',
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
  letterSpacing: '.08em',
  textTransform: 'uppercase' as const,
  padding: 'var(--sp-1) var(--sp-4)',
  textDecoration: 'none',
  whiteSpace: 'nowrap',
  transition: 'color .2s cubic-bezier(0.22, 1, 0.36, 1), background .2s cubic-bezier(0.22, 1, 0.36, 1), border-color .2s cubic-bezier(0.22, 1, 0.36, 1), transform .2s cubic-bezier(0.22, 1, 0.36, 1), box-shadow .2s cubic-bezier(0.22, 1, 0.36, 1)',
  transform: 'translateY(0)',
};

const linkHoverHandlers = {
  onMouseEnter: (e: React.MouseEvent<HTMLAnchorElement>) => {
    e.currentTarget.style.transform = 'translateY(-1px)';
  },
  onMouseLeave: (e: React.MouseEvent<HTMLAnchorElement>) => {
    e.currentTarget.style.transform = 'translateY(0)';
  },
};

const activeLinkStyle: CSSProperties = {
  color: 'var(--rose-bright)',
  background: 'var(--rose-deep)',
  borderColor: 'var(--rose-dim)',
  boxShadow: '0 0 12px rgba(220,165,189,.15)',
};

const bodyStyle: CSSProperties = {
  flex: 1,
  minHeight: 0,
  padding: 'var(--sp-2) var(--sp-4) var(--sp-4)',
  overflowY: 'auto',
};

export default function DashboardLayout() {
  const { pathname } = useLocation();

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
            {...linkHoverHandlers}
          >
            <FlatIcon name={view.icon} size={13} tone="muted" />
            {view.label}
          </NavLink>
        ))}
      </nav>
      <div style={bodyStyle}>
        <RevealWhen key={pathname} visible mode="slide-up" duration={300}>
          <Outlet />
        </RevealWhen>
      </div>
    </div>
  );
}
