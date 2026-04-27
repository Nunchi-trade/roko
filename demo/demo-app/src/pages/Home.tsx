import { Link } from 'react-router';
import { useServerHealth } from '../hooks/useServerHealth';
import './Home.css';

const LINKS = [
  {
    section: 'demo',
    items: [
      { to: '/demo', label: 'Unified Demo', desc: 'Series A pitch demo — 7 scenarios, live terminals, ROSEDUST visual system' },
      { to: '/bench', label: 'Benchmark Lab', desc: 'Configure and run SWE-bench evaluations, compare models, track self-learning' },
      { to: '/bench-live', label: 'Live Monitor', desc: 'Real-time benchmark observation — task grid, cost chart, activity feed' },
      { to: '/builder', label: 'Builder', desc: 'Type a request — roko builds it live in a temp repo' },
      { to: '/terminal', label: 'Terminal', desc: 'Multi-pane browser terminal with real PTY sessions' },
    ],
  },
  {
    section: 'explore',
    items: [
      { to: '/explorer', label: 'Explorer', desc: 'Browse health, status, episodes, and StateHub events with filters and detail views' },
    ],
  },
  {
    section: 'api',
    items: [
      { href: '/api/health', label: 'Health', desc: 'Server health check' },
      { href: '/api/status', label: 'Status', desc: 'Workspace status and signal counts' },
      { href: '/api/episodes', label: 'Episodes', desc: 'Agent execution episodes' },
      { href: '/api/terminal/sessions', label: 'Terminal Sessions', desc: 'Active PTY session list' },
    ],
  },
  {
    section: 'docs',
    items: [
      { href: '/api/openapi.json', label: 'OpenAPI Spec', desc: 'Full API schema (JSON)' },
    ],
  },
];

export default function Home() {
  const health = useServerHealth();

  return (
    <div className="home-page">
      <h1 className="home-title">◆ roko serve</h1>
      <p className="home-sub">agent runtime control plane</p>
      <div className="home-status">
        <span className={`home-dot ${health}`} />
        <span>{health}</span>
      </div>
      <div className="home-links">
        {LINKS.map((group) => (
          <div key={group.section}>
            <div className="home-section">{group.section}</div>
            {group.items.map((item) => {
              const inner = (
                <>
                  <span className="link-label">{item.label}</span>
                  <div className="link-desc">{item.desc}</div>
                </>
              );
              if ('to' in item) {
                return <Link key={item.to} to={item.to} className="home-link">{inner}</Link>;
              }
              return <a key={item.href} href={item.href} className="home-link">{inner}</a>;
            })}
          </div>
        ))}
      </div>
    </div>
  );
}
