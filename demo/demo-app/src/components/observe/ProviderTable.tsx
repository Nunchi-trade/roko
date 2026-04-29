import { AnimatedRow, TableEmptyState } from '../AnimatedTable';
import './ProviderTable.css';

export interface ProviderTableProps {
  providers: Record<string, { healthy: boolean; latency_ms?: number }>;
}

/**
 * Tabular provider health display with animated rows.
 * Extracted from Explorer.tsx bottom drawer provider section.
 */
export function ProviderTable({ providers }: ProviderTableProps) {
  const entries = Object.entries(providers);

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
        {entries.length === 0 ? (
          <TableEmptyState colSpan={3} message="No providers configured" />
        ) : (
          entries.map(([name, info], i) => {
            const status = info.healthy ? 'healthy' : 'down';
            return (
              <AnimatedRow key={name} index={i}>
                <td className="provider-table__name">{name}</td>
                <td>
                  <span className={`provider-table__status provider-table__status--${status}`}>
                    {status}
                  </span>
                </td>
                <td className="provider-table__latency">
                  {info.latency_ms != null ? `${info.latency_ms}ms` : '--'}
                </td>
              </AnimatedRow>
            );
          })
        )}
      </tbody>
    </table>
  );
}
