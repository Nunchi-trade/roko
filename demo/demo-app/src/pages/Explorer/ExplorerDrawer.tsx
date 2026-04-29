import FlatIcon, { inferIcon } from '../../components/FlatIcon';
import type { StateEvent } from './types';

function safeTimestamp(ts: unknown): string {
  if (!ts) return '';
  try {
    const d = new Date(ts as string | number);
    return isNaN(d.getTime()) ? '' : d.toLocaleTimeString();
  } catch {
    return '';
  }
}

function safePayload(payload: unknown): string {
  try {
    const s = JSON.stringify(payload);
    return s.length > 140 ? s.slice(0, 140) + '...' : s;
  } catch {
    return String(payload ?? '');
  }
}

interface ExplorerDrawerProps {
  drawerOpen: boolean;
  onToggle: () => void;
  provEntries: [string, { healthy: boolean }][];
  events: StateEvent[];
}

export default function ExplorerDrawer({
  drawerOpen,
  onToggle,
  provEntries,
  events,
}: ExplorerDrawerProps) {
  return (
    <div className={`expl-drawer${drawerOpen ? ' open' : ''}`}>
      <button
        className="expl-drawer-handle btn-ghost-reveal"
        onClick={onToggle}
        aria-label={drawerOpen ? 'Collapse drawer' : 'Expand drawer'}
      >
        <span className="expl-drawer-bar" />
        <span className="expl-drawer-hint">{drawerOpen ? 'Collapse' : 'Providers & Events'}</span>
      </button>

      <div className="expl-drawer-body">
        {/* Left: Provider health */}
        <div className="expl-drawer-providers">
          <div className="expl-section-label"><FlatIcon name="provider" size={15} tone="success" />PROVIDER HEALTH</div>
          {provEntries.length === 0 ? (
            <div className="expl-empty">No providers configured</div>
          ) : (
            <div className="expl-provider-list">
              {provEntries.map(([name, info]) => (
                <div key={name} className={`provider-card${info.healthy ? ' provider-card--healthy' : ''}`}>
                  <FlatIcon name="provider" size={14} tone={info.healthy ? 'success' : 'warning'} />
                  <span className="provider-name">{name}</span>
                  <span className={`provider-badge ${info.healthy ? 'ok' : 'down'}`}>
                    {info.healthy ? 'ok' : 'down'}
                  </span>
                </div>
              ))}
            </div>
          )}
        </div>

        {/* Right: Event stream */}
        <div className="expl-drawer-events">
          <div className="expl-section-label"><FlatIcon name="event" size={15} tone="dream" />EVENT STREAM</div>
          <div className="expl-event-list">
            {events.length === 0 ? (
              <div className="expl-empty">No events recorded yet</div>
            ) : (
              events.slice(0, 16).map((evt, i) => (
                <div key={`${evt?.type ?? 'evt'}-${i}`} className="expl-event-item" style={{ animationDelay: `${i * 50}ms` }}>
                  <span className="expl-event-ts">{safeTimestamp(evt?.timestamp)}</span>
                  <span className="expl-event-badge"><FlatIcon name={inferIcon(evt?.type ?? 'event')} size={12} tone="muted" />{evt?.type ?? 'unknown'}</span>
                  <span className="expl-event-payload">{safePayload(evt?.payload)}</span>
                </div>
              ))
            )}
          </div>
        </div>
      </div>
    </div>
  );
}
