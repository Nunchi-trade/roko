import { useEffect } from 'react';
import { useDataHub } from '../app/DataHub';
import { useShallow } from 'zustand/react/shallow';

export function useAgentData() {
  const { agents, fetchAgents } = useDataHub(
    useShallow((s) => ({
      agents: s.agents,
      fetchAgents: s.fetchAgents,
    })),
  );
  useEffect(() => {
    fetchAgents();
  }, [fetchAgents]);
  return agents;
}
