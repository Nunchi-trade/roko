import { useDataHub } from '../app/DataHub';

export function useEpisodes() {
  return useDataHub((s) => s.episodes);
}
