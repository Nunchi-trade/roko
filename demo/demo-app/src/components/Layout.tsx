import { NavLink, Outlet } from 'react-router';
import { useServerHealth } from '../hooks/useServerHealth';
import './Layout.css';

const NAV_ITEMS = [
  { to: '/', label: 'Home', num: '0' },
  { to: '/demo', label: 'Demo', num: '1' },
  { to: '/terminal', label: 'Terminal', num: '2' },
  { to: '/builder', label: 'Builder', num: '3' },
  { to: '/explorer', label: 'Explorer', num: '4' },
  { to: '/bench', label: 'Bench', num: '5' },
  { to: '/bench-live', label: 'Live', num: '6' },
];

export default function Layout() {
  const health = useServerHealth();

  return (
    <div className="layout">
      <header className="topbar">
        <NavLink to="/" className="logo">◆ roko</NavLink>
        <nav className="tabs">
          {NAV_ITEMS.map((item) => (
            <NavLink
              key={item.to}
              to={item.to}
              end={item.to === '/'}
              className={({ isActive }) => `tab${isActive ? ' active' : ''}`}
            >
              <span className="tab-num">{item.num}</span>
              {item.label}
            </NavLink>
          ))}
        </nav>
        <div className="spacer" />
        <div className="status-indicator">
          <span className={`status-dot ${health}`} />
          <span className="status-label">{health}</span>
        </div>
      </header>
      <main className="content">
        <Outlet />
      </main>
    </div>
  );
}
