import './Tabs.css';

interface Tab {
  id: string;
  label: string;
  badge?: number;
}

interface TabsProps {
  tabs: Tab[];
  active: string;
  onChange: (id: string) => void;
  className?: string;
}

export function Tabs({ tabs, active, onChange, className }: TabsProps) {
  return (
    <div className={`tabs${className ? ` ${className}` : ''}`}>
      {tabs.map((tab) => (
        <button
          key={tab.id}
          className={`tabs__tab${tab.id === active ? ' tabs__tab--active' : ''}`}
          onClick={() => onChange(tab.id)}
          type="button"
        >
          <span className="tabs__label">{tab.label}</span>
          {tab.badge != null && tab.badge > 0 && (
            <span className="tabs__badge">{tab.badge}</span>
          )}
        </button>
      ))}
    </div>
  );
}
