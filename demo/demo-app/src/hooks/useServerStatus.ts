import { useDataHub } from '../app/DataHub';

export function useServerStatus() {
  return useDataHub((s) => s.serverStatus);
}
