/**
 * Feed catalog API client.
 *
 * Typed wrappers for `/api/feeds/catalog`.
 */

export interface FeedCatalogResponse {
  agents: FeedCatalogAgent[];
  feeds: FeedCatalogFeed[];
  stats: FeedCatalogStats;
}

export interface FeedCatalogAgent {
  agent_id: string;
  name: string;
  capabilities: string[];
  feed_count: number;
  online: boolean;
}

export interface FeedCatalogFeed {
  feed_id: string;
  topic: string;
  name: string;
  description: string;
  kind: string;
  rate: string;
  agent_id: string;
}

export interface FeedCatalogStats {
  total_agents: number;
  total_feeds: number;
  messages_per_sec: number;
}

/** Format basis points as a percentage string (e.g. 620 → "6.20%"). */
export function formatBps(bps: number): string {
  return `${(bps / 100).toFixed(2)}%`;
}

/** Format a gwei value with one decimal. */
export function formatGwei(gwei: number): string {
  return `${gwei.toFixed(1)} gwei`;
}
