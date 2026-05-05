import { useEffect } from 'react';
import { useDataHub } from '../app/DataHub';
import { useShallow } from 'zustand/react/shallow';

export function useConfig() {
  const slice = useDataHub(
    useShallow((s) => ({
      config: s.config,
      defaultModel: s.defaultModel,
      defaultBackend: s.defaultBackend,
      fetchConfig: s.fetchConfig,
      updateConfig: s.updateConfig,
    })),
  );
  useEffect(() => {
    if (!slice.config) slice.fetchConfig();
  }, [slice.config, slice.fetchConfig]);
  return {
    config: slice.config,
    defaultModel: slice.defaultModel,
    defaultBackend: slice.defaultBackend,
    updateConfig: slice.updateConfig,
    refreshConfig: slice.fetchConfig,
  };
}
