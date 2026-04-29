import { createContext, createElement, useContext, useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useApiWithFallback } from './useApiWithFallback';

/** A provider→models grouping derived from config.models */
export interface ProviderGroup {
  provider: string;
  kind: string;
  models: { name: string; slug: string }[];
}

export interface RokoConfigState {
  defaultModel: string;
  defaultBackend: string;
  providers: ProviderGroup[];
  isLive: boolean;
  lastSaved: number | null;
  updateModelConfig: (model: string, backend: string) => Promise<boolean>;
}

const FALLBACK: RokoConfigState = {
  defaultModel: '',
  defaultBackend: '',
  providers: [],
  isLive: false,
  lastSaved: null,
  updateModelConfig: async () => false,
};

export const RokoConfigContext = createContext<RokoConfigState>(FALLBACK);

export function useRokoConfig() {
  return useContext(RokoConfigContext);
}

/** Derive provider→model groups from the config.models map */
function deriveProviders(
  models: Record<string, { provider: string; slug: string }> | undefined,
  providers: Record<string, { kind: string }> | undefined,
): ProviderGroup[] {
  if (!models) return [];
  const grouped = new Map<string, ProviderGroup>();
  for (const [name, m] of Object.entries(models)) {
    let group = grouped.get(m.provider);
    if (!group) {
      group = {
        provider: m.provider,
        kind: providers?.[m.provider]?.kind ?? 'unknown',
        models: [],
      };
      grouped.set(m.provider, group);
    }
    group.models.push({ name, slug: m.slug });
  }
  return Array.from(grouped.values());
}

/** Hook that manages fetching + polling + writing config. Used inside RokoConfigProvider. */
export function useRokoConfigState(): RokoConfigState {
  const { get, put, isLive } = useApiWithFallback();
  const [defaultModel, setDefaultModel] = useState('');
  const [defaultBackend, setDefaultBackend] = useState('');
  const [providers, setProviders] = useState<ProviderGroup[]>([]);
  const [lastSaved, setLastSaved] = useState<number | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval>>(undefined);

  const fetchConfig = useCallback(async () => {
    try {
      const cfg = await get<Record<string, unknown>>('/api/config');
      const agent = cfg?.agent as Record<string, string> | undefined;
      if (agent?.default_model) setDefaultModel(agent.default_model);
      if (agent?.default_backend) setDefaultBackend(agent.default_backend);
      setProviders(
        deriveProviders(
          cfg?.models as Record<string, { provider: string; slug: string }>,
          cfg?.providers as Record<string, { kind: string }>,
        ),
      );
    } catch {
      // swallow — fallback data will be used
    }
  }, [get]);

  // Initial fetch + 15s poll
  useEffect(() => {
    fetchConfig();
    intervalRef.current = setInterval(fetchConfig, 15_000);
    return () => clearInterval(intervalRef.current);
  }, [fetchConfig]);

  const updateModelConfig = useCallback(
    async (model: string, backend: string): Promise<boolean> => {
      if (!isLive) return false;
      try {
        const cfg = await put<Record<string, unknown>>('/api/config', {
          agent: { default_model: model, default_backend: backend },
        });
        // Update from response
        const agent = cfg?.agent as Record<string, string> | undefined;
        if (agent?.default_model) setDefaultModel(agent.default_model);
        if (agent?.default_backend) setDefaultBackend(agent.default_backend);
        setLastSaved(Date.now());
        return true;
      } catch {
        return false;
      }
    },
    [isLive, put],
  );

  return { defaultModel, defaultBackend, providers, isLive, lastSaved, updateModelConfig };
}

/** Context provider — wrap in AppShell so all pages can access config */
export function RokoConfigProvider({ children }: { children: ReactNode }) {
  const value = useRokoConfigState();
  return createElement(RokoConfigContext.Provider, { value }, children);
}
