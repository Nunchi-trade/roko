/**
 * Zustand selector utilities for preventing unnecessary re-renders.
 *
 * When selecting multiple fields from a Zustand store, the default reference
 * equality check causes re-renders on ANY store change. These helpers use
 * shallow comparison so components only re-render when their selected fields
 * actually change.
 *
 * Usage:
 *   import { useShallowStore } from '../lib/selector-utils';
 *   import { useMyStore } from '../stores/MyStore';
 *
 *   // Instead of:
 *   //   const { plans, agents } = useMyStore(s => ({ plans: s.plans, agents: s.agents }));
 *   //
 *   // Use:
 *   const { plans, agents } = useShallowStore(useMyStore, s => ({
 *     plans: s.plans,
 *     agents: s.agents,
 *   }));
 */

import { useShallow } from 'zustand/react/shallow';

/**
 * A typed helper that wraps a Zustand store hook with shallow comparison.
 *
 * @param useStore  The Zustand hook (e.g. useDataHub)
 * @param selector  A function selecting a slice of the store state
 * @returns The selected slice, re-rendering only on shallow-diff changes
 *
 * @example
 *   const { tasks, errors } = useShallowStore(useDataHub, (s) => ({
 *     tasks: s.tasks,
 *     errors: s.errors,
 *   }));
 */
export function useShallowStore<TState, TSlice>(
  useStore: (selector: (state: TState) => TSlice) => TSlice,
  selector: (state: TState) => TSlice,
): TSlice {
  return useStore(useShallow(selector));
}

/**
 * Create a reusable shallow selector for a specific store.
 * Useful when the same multi-field selection is used in several components.
 *
 * @example
 *   // In a shared selectors file:
 *   export const usePlanData = createShallowSelector(useDataHub, (s) => ({
 *     plans: s.plans,
 *     activePlan: s.activePlan,
 *   }));
 *
 *   // In a component:
 *   const { plans, activePlan } = usePlanData();
 */
export function createShallowSelector<TState, TSlice>(
  useStore: (selector: (state: TState) => TSlice) => TSlice,
  selector: (state: TState) => TSlice,
): () => TSlice {
  return () => useStore(useShallow(selector));
}
