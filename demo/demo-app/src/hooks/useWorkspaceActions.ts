import { useDataHub } from '../app/DataHub';
import { useShallow } from 'zustand/react/shallow';

export function useWorkspaceActions() {
  return useDataHub(
    useShallow((s) => ({
      serverWorkdir: s.serverWorkdir,
      workspace: s.workspace,
      ensureWorkspace: s.ensureWorkspace,
      destroyWorkspace: s.destroyWorkspace,
    })),
  );
}
