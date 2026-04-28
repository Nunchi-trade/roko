import { useEffect } from 'react';
import { useApiWithFallback } from './useApiWithFallback';

const SEED_KEY = 'roko-seeded';

export function useSeedData() {
  const { post, isLive } = useApiWithFallback();

  useEffect(() => {
    if (!isLive) return;
    if (sessionStorage.getItem(SEED_KEY)) return;

    (async () => {
      try {
        await Promise.all([
          post('/api/agents/register', { name: 'rustsmith', capabilities: ['rust', 'systems', 'testing'], reputation: 92, model: 'claude-sonnet' }),
          post('/api/agents/register', { name: 'ethdev', capabilities: ['solidity', 'evm', 'defi'], reputation: 88, model: 'claude-sonnet' }),
          post('/api/agents/register', { name: 'fullstack', capabilities: ['typescript', 'react', 'api'], reputation: 85, model: 'gpt-4o' }),
          post('/api/agents/register', { name: 'researcher', capabilities: ['research', 'papers', 'docs'], reputation: 90, model: 'claude-haiku' }),
          post('/api/agents/register', { name: 'auditor', capabilities: ['security', 'audit', 'review'], reputation: 95, model: 'claude-opus' }),
        ]);
        await post('/api/jobs', { title: 'Implement cascade routing optimization', priority: 'high' });
        sessionStorage.setItem(SEED_KEY, '1');
      } catch {
        // silently swallow errors
      }
    })();
  }, [isLive]); // eslint-disable-line react-hooks/exhaustive-deps
}
