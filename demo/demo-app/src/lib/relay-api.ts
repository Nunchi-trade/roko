/**
 * Relay API — types mirroring agent-relay protocol.rs + fetch helpers.
 */
import { api } from '../transport/api';

// ── Types ────────────────────────────────────────────────────

export interface ConnectedAgent {
  agent_id: string;
  name?: string;
  capabilities: string[];
  rest_endpoint?: string;
  card_uri?: string;
  connected_at_ms: number;
  relay_backed: boolean;
}

export interface ConnectedWorkspace {
  workspace_id: string;
  name?: string;
  url: string;
  version?: string;
  owner_wallet?: string;
  agents_count: number;
  connected_at_ms: number;
  last_heartbeat_ms: number;
}

export interface FeedDescriptor {
  feed_id: string;
  topic: string;
  name: string;
  description: string;
  kind: string;
  rate: string;
  schema?: unknown;
}

export interface TopicInfo {
  topic: string;
  /** Subscriber count (number) or list of subscriber IDs (string[]). */
  subscribers: number | string[];
}

export interface TopicEnvelope {
  topic: string;
  msg_type: string;
  payload: unknown;
  publisher_id?: string;
  seq: number;
  timestamp_ms: number;
}

export type RelayEvent =
  | { type: 'agent_connected'; agent: ConnectedAgent }
  | { type: 'agent_disconnected'; agent_id: string }
  | { type: 'workspace_connected'; workspace: ConnectedWorkspace }
  | { type: 'workspace_disconnected'; workspace_id: string }
  | { type: 'workspace_heartbeat'; workspace_id: string; agents_count: number }
  | { type: 'feed_registered'; agent_id: string; feed: FeedDescriptor }
  | { type: 'feed_unregistered'; agent_id: string; feed_id: string }
  | { type: 'card_updated'; agent_id: string; card_uri: string }
  | { type: 'message_delivered'; agent_id: string; message_id: string }
  | { type: 'message_responded'; agent_id: string; message_id: string }
  | { type: 'agent_error'; agent_id: string; message_id?: string; error: string };

// ── Fetch helpers ────────────────────────────────────────────

export function fetchRelayAgents() {
  return api.get<ConnectedAgent[]>('/relay/agents');
}

export function fetchRelayWorkspaces() {
  return api.get<ConnectedWorkspace[]>('/relay/workspaces');
}

export function fetchRelayFeeds() {
  return api.get<Record<string, FeedDescriptor[]>>('/relay/feeds');
}

export async function fetchRelayTopics() {
  // The relay returns { topics: [...] } — unwrap the envelope.
  const res = await api.get<TopicInfo[] | { topics: TopicInfo[] }>('/relay/topics');
  if (res.ok) {
    const data = Array.isArray(res.data) ? res.data : (res.data as { topics: TopicInfo[] }).topics ?? [];
    return { ok: true as const, data };
  }
  return res;
}

export function fetchTopicMessages(topic: string, limit = 50) {
  return api.get<TopicEnvelope[]>(`/relay/topics/${encodeURIComponent(topic)}/messages?limit=${limit}`);
}

export function fetchRelayHealth() {
  return api.get<{ status: string; agents: number; workspaces: number; topics: number }>('/relay/health');
}
