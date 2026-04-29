import './ProviderTable.css';

export interface ProviderTableProps {
  providers: Record<string, { healthy: boolean; latency_ms?: number }>;
}

/**
 * Tabular provider health display.
 * Extracted from Explorer.tsx bottom drawer provider section.
 */
export function ProviderTable({ providers }: ProviderTableProps) {
  const entries = Object.entries(providers);
  if (entries.length === 0) {
    return <div className="provider-table__empty">No providers configured</div>;
  }

  return (
    <table className="provider-table">
      <thead>
        <tr>
          <th>Provider</th>
          <th>Status</th>
          <th>Latency</th>
        </tr>
      </thead>
      <tbody>
        {entries.map(([name, info]) => {
          const status = info.healthy ? 'healthy' : 'down';
          return (
            <tr key={name}>
              <td className="provider-table__name">{name}</td>
              <td>
                <span className={`provider-table__status provider-table__status--${status}`}>
                  {status}
                </span>
              </td>
              <td className="provider-table__latency">
                {info.latency_ms != null ? `${info.latency_ms}ms` : '--'}
              </td>
            </tr>
          );
        })}
      </tbody>
    </table>
  );
}
