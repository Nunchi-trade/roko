import { useState, useCallback, useRef } from 'react';
import type { CommandDef } from '../lib/scenarios';

export interface CommandState {
  id: string;
  command: string;
  description: string;
  status: 'pending' | 'running' | 'success' | 'failure';
  elapsed?: number;
  error?: string;
}

export function useCommandList(commands: CommandDef[]) {
  const [items, setItems] = useState<CommandState[]>(
    commands.map(c => ({ ...c, status: 'pending' as const })),
  );
  const startTime = useRef<number>(0);

  const markRunning = useCallback((id: string) => {
    startTime.current = Date.now();
    setItems(prev => prev.map(item =>
      item.id === id ? { ...item, status: 'running' as const } : item,
    ));
  }, []);

  const markSuccess = useCallback((id: string) => {
    const elapsed = Date.now() - startTime.current;
    setItems(prev => prev.map(item =>
      item.id === id ? { ...item, status: 'success' as const, elapsed } : item,
    ));
  }, []);

  const markFailure = useCallback((id: string, error?: string) => {
    const elapsed = Date.now() - startTime.current;
    setItems(prev => prev.map(item =>
      item.id === id ? { ...item, status: 'failure' as const, elapsed, error } : item,
    ));
  }, []);

  const reset = useCallback(() => {
    setItems(commands.map(c => ({ ...c, status: 'pending' as const })));
  }, [commands]);

  const nextPendingId = items.find(i => i.status === 'pending')?.id;
  const isRunning = items.some(i => i.status === 'running');

  return { items, markRunning, markSuccess, markFailure, reset, nextPendingId, isRunning };
}
