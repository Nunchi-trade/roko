import { createContext, createElement, useContext, useCallback, useEffect, useRef, useState } from 'react';
import type { ReactNode } from 'react';
import { useLiveApi } from './useLiveApi';
import {
  providerForModelKey,
  rawModelsToOptions,
  resolveModelKey,
  type RawConfigModels,
} from '../lib/config-models';

/** A provider→models grouping derived from config.models */
export interface ProviderGroup {
  provider: string;
  kind: string;
  models: { key: string; name: string; slug: string }[];
}

export interface RokoConfigState {
  fullConfig: Record<string, unknown>;
  defaultModel: string;
  defaultBackend: string;
  providers: ProviderGroup[];
  isLive: boolean;
  lastSaved: number | null;
  updateModelConfig: (model: string, backend: string) => Promise<boolean>;
  updateConfig: (partial: Record<string, unknown>) => Promise<boolean>;
  refreshConfig: () => Promise<void>;
}

const DEFAULT_CONFIG_STATE: RokoConfigState = {
  fullConfig: {},
  defaultModel: '',
  defaultBackend: '',
  providers: [],
  isLive: false,
  lastSaved: null,
  updateModelConfig: async () => false,
  updateConfig: async () => false,
  refreshConfig: async () => {},
};

export const RokoConfigContext = createContext<RokoConfigState>(DEFAULT_CONFIG_STATE);

export function useRokoConfig() {
  return useContext(RokoConfigContext);
}

/** Derive provider→model groups from the config.models map */
function deriveProviders(
  models: RawConfigModels | undefined,
  providers: Record<string, { kind: string }> | undefined,
): ProviderGroup[] {
  if (!models) return [];
  const grouped = new Map<string, ProviderGroup>();
  for (const [key, m] of Object.entries(models)) {
    let group = grouped.get(m.provider);
    if (!group) {
      group = {
        provider: m.provider,
        kind: providers?.[m.provider]?.kind ?? 'unknown',
        models: [],
      };
      grouped.set(m.provider, group);
    }
    group.models.push({ key, name: key, slug: m.slug });
  }
  return Array.from(grouped.values());
}

/** Apply a config blob to local state (model, backend, providers, fullConfig). */
function applyConfig(
  cfg: Record<string, unknown>,
  setFullConfig: (c: Record<string, unknown>) => void,
  setDefaultModel: (m: string) => void,
  setDefaultBackend: (b: string) => void,
  setProviders: (p: ProviderGroup[]) => void,
) {
  setFullConfig(cfg);
  const agent = cfg?.agent as Record<string, string> | undefined;
  const rawModels = cfg?.models as RawConfigModels | undefined;
  const modelOptions = rawModelsToOptions(rawModels);
  const modelKey = agent?.default_model
    ? resolveModelKey(modelOptions, agent.default_model)
    : '';
  if (modelKey) setDefaultModel(modelKey);
  const modelProvider = providerForModelKey(modelOptions, modelKey);
  if (modelProvider || agent?.default_backend) {
    setDefaultBackend(modelProvider ?? agent?.default_backend ?? '');
  }
  setProviders(deriveProviders(rawModels, cfg?.providers as Record<string, { kind: string }>));
}

/** Hook that manages fetching + polling + writing config. Used inside RokoConfigProvider. */
export function useRokoConfigState(): RokoConfigState {
  const { get, put, isLive } = useLiveApi();
  const [fullConfig, setFullConfig] = useState<Record<string, unknown>>({});
  const [defaultModel, setDefaultModel] = useState('');
  const [defaultBackend, setDefaultBackend] = useState('');
  const [providers, setProviders] = useState<ProviderGroup[]>([]);
  const [lastSaved, setLastSaved] = useState<number | null>(null);
  const intervalRef = useRef<ReturnType<typeof setInterval>>(undefined);

  const fetchConfig = useCallback(async () => {
    try {
      const cfg = await get<Record<string, unknown>>('/api/config');
      if (cfg) applyConfig(cfg, setFullConfig, setDefaultModel, setDefaultBackend, setProviders);
    } catch {
      // Leave config empty until the live server responds.
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
        const allModels = providers.flatMap((provider) =>
          provider.models.map((providerModel) => ({
            key: providerModel.key,
            name: providerModel.name,
            slug: providerModel.slug,
            provider: provider.provider,
          })),
        );
        const modelKey = resolveModelKey(allModels, model);
        const modelBackend = providerForModelKey(allModels, modelKey) ?? backend;
        const cfg = await put<Record<string, unknown>>('/api/config', {
          agent: { default_model: modelKey, default_backend: modelBackend },
        });
        if (cfg) applyConfig(cfg, setFullConfig, setDefaultModel, setDefaultBackend, setProviders);
        setLastSaved(Date.now());
        return true;
      } catch {
        return false;
      }
    },
    [isLive, providers, put],
  );

  const updateConfig = useCallback(
    async (partial: Record<string, unknown>): Promise<boolean> => {
      if (!isLive) return false;
      try {
        const cfg = await put<Record<string, unknown>>('/api/config', partial);
        if (cfg) applyConfig(cfg, setFullConfig, setDefaultModel, setDefaultBackend, setProviders);
        setLastSaved(Date.now());
        return true;
      } catch {
        return false;
      }
    },
    [isLive, put],
  );

  const refreshConfig = useCallback(async () => {
    await fetchConfig();
  }, [fetchConfig]);

  return {
    fullConfig, defaultModel, defaultBackend, providers, isLive, lastSaved,
    updateModelConfig, updateConfig, refreshConfig,
  };
}

/** Context provider — wrap in AppShell so all pages can access config */
export function RokoConfigProvider({ children }: { children: ReactNode }) {
  const value = useRokoConfigState();
  return createElement(RokoConfigContext.Provider, { value }, children);
}
