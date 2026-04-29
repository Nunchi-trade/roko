import { useEffect } from 'react';
import { useDataHub } from '../app/DataHub';
import { useShallow } from 'zustand/react/shallow';

export function useBenchData() {
  const slice = useDataHub(
    useShallow((s) => ({
      benchRuns: s.benchRuns,
      benchSuites: s.benchSuites,
      benchModels: s.benchModels,
      fetchBenchRuns: s.fetchBenchRuns,
      fetchBenchSuites: s.fetchBenchSuites,
      fetchBenchModels: s.fetchBenchModels,
    })),
  );
  useEffect(() => {
    slice.fetchBenchRuns();
    slice.fetchBenchSuites();
    slice.fetchBenchModels();
  }, [slice.fetchBenchRuns, slice.fetchBenchSuites, slice.fetchBenchModels]);
  return {
    benchRuns: slice.benchRuns,
    benchSuites: slice.benchSuites,
    benchModels: slice.benchModels,
  };
}
