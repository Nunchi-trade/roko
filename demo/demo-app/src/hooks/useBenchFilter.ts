/**
 * useBenchFilter — manages sort, filter, and search state for run lists.
 * Extracted from useBench.ts. Provides pure filter/sort logic.
 */
import { useState, useMemo } from 'react';
import type { BenchRun } from '../lib/bench-types';

/* ── Types ── */

type SortField = 'date' | 'passRate' | 'cost' | 'model';
type SortDir = 'asc' | 'desc';

export interface UseBenchFilterReturn {
  sortField: SortField;
  sortDir: SortDir;
  search: string;
  setSortField: (f: SortField) => void;
  setSortDir: (d: SortDir) => void;
  setSearch: (s: string) => void;
  filterRuns: (runs: BenchRun[]) => BenchRun[];
}

/* ── Hook ── */

export function useBenchFilter(): UseBenchFilterReturn {
  const [sortField, setSortField] = useState<SortField>('date');
  const [sortDir, setSortDir] = useState<SortDir>('desc');
  const [search, setSearch] = useState('');

  const filterRuns = useMemo(
    () => (runs: BenchRun[]) => {
      let result = [...runs];
      if (search) {
        const q = search.toLowerCase();
        result = result.filter(
          (r) =>
            r.suite_name.toLowerCase().includes(q) ||
            r.config.model.toLowerCase().includes(q),
        );
      }
      result.sort((a, b) => {
        const dir = sortDir === 'asc' ? 1 : -1;
        switch (sortField) {
          case 'date':
            return (
              dir *
              (new Date(a.started_at).getTime() -
                new Date(b.started_at).getTime())
            );
          case 'passRate':
            return (
              dir *
              ((a.summary?.pass_rate ?? 0) - (b.summary?.pass_rate ?? 0))
            );
          case 'cost':
            return (
              dir *
              ((a.summary?.total_cost_usd ?? 0) -
                (b.summary?.total_cost_usd ?? 0))
            );
          case 'model':
            return dir * a.config.model.localeCompare(b.config.model);
          default:
            return 0;
        }
      });
      return result;
    },
    [sortField, sortDir, search],
  );

  return {
    sortField,
    sortDir,
    search,
    setSortField,
    setSortDir,
    setSearch,
    filterRuns,
  };
}
