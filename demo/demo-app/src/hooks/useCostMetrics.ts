import { useDataHub } from '../app/DataHub';
import { useShallow } from 'zustand/react/shallow';

export function useCostMetrics() {
  return useDataHub(
    useShallow((s) => ({
      totalCost: s.totalCost,
      totalTokens: s.totalTokens,
      recentInferences: s.recentInferences,
    })),
  );
}
