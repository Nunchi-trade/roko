/**
 * Chain API client — typed wrappers around roko-serve chain endpoints.
 *
 * Uses the singleton `api` from transport/api.ts.
 */

import { api } from '../transport/api';

// ── Types ─────────────────────────────────────────────────────────

export interface ChainBlock {
  number: number;
  hash: string;
  parent_hash: string;
  timestamp: number;
  gas_used: number;
  gas_limit: number;
  tx_count: number;
  base_fee_per_gas: number | null;
}

export interface ChainTx {
  block_number: number;
  tx_hash: string;
  from: string;
  to: string | null;
  value_wei: string;
  gas_used: number;
  method_sig: string | null;
  success: boolean;
}

export interface ChainContractEvent {
  block_number: number;
  tx_hash: string;
  log_index: number;
  contract: string;
  event_name: string;
  decoded: Record<string, unknown>;
}

export interface ChainWatcherStatus {
  watcher_running: boolean;
  latest_block: number | null;
  blocks_buffered: number;
  txs_buffered: number;
  events_buffered: number;
}

// ── API calls ─────────────────────────────────────────────────────

export async function fetchChainBlocks(limit = 64): Promise<ChainBlock[]> {
  const res = await api.get<{ blocks: ChainBlock[] }>(`/api/chain/blocks?limit=${limit}`);
  return res.ok ? res.data.blocks : [];
}

export async function fetchChainTxs(limit = 128): Promise<ChainTx[]> {
  const res = await api.get<{ transactions: ChainTx[] }>(`/api/chain/transactions?limit=${limit}`);
  return res.ok ? res.data.transactions : [];
}

export async function fetchChainEvents(limit = 128): Promise<ChainContractEvent[]> {
  const res = await api.get<{ events: ChainContractEvent[] }>(`/api/chain/events?limit=${limit}`);
  return res.ok ? res.data.events : [];
}

export async function fetchChainWatcherStatus(): Promise<ChainWatcherStatus | null> {
  const res = await api.get<ChainWatcherStatus>('/api/chain/watcher');
  return res.ok ? res.data : null;
}
