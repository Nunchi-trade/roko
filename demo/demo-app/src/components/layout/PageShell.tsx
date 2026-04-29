import type { ReactNode } from 'react';
import './PageShell.css';

interface PageShellProps {
  title: string;
  subtitle?: string;
  actions?: ReactNode;
  children: ReactNode;
  className?: string;
}

export function PageShell({ title, subtitle, actions, children, className }: PageShellProps) {
  return (
    <div className={`page-shell${className ? ` ${className}` : ''}`}>
      <div className="page-shell__header">
        <div className="page-shell__titles">
          <h2 className="page-shell__title">{title}</h2>
          {subtitle && <p className="page-shell__subtitle">{subtitle}</p>}
        </div>
        {actions && <div className="page-shell__actions">{actions}</div>}
      </div>
      <div className="page-shell__content">
        {children}
      </div>
    </div>
  );
}
