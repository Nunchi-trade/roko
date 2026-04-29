import { useMemo, useEffect } from 'react';
import { useChainWs, type ChainWsState, type PheromoneEvent } from './useChain';
import type { BlockData, TxData } from '../components/ChainActivityPanel';
import type { AgentPosition } from '../components/LivePositionsPanel';

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

export interface ChainData {
  /** Whether the WS connection to mirage-rs is established. */
  connected: boolean;
  /** Derived blocks from pheromone deposits. */
  blocks: BlockData[];
  /** Agent positions with live-updated key metrics. */
  positions: AgentPosition[];
  /** Raw WS state for passthrough to other components. */
  ws: ChainWsState;
}

// ---------------------------------------------------------------------------
// Default positions (mirrored from Demo.tsx static state)
// ---------------------------------------------------------------------------

const DEFAULT_POSITIONS: AgentPosition[] = [
  {
    name: 'Yield Scout',
    address: '0x70997970C51812dc3A010C7d01b50e0d17dc79C8',
    color: 'rose',
    balances: [
      { token: 'ETH', amount: 10, decimals: 4 },
      { token: 'USDC', amount: 500000, decimals: 2 },
    ],
    keyMetric: { label: 'APR', value: '--' },
  },
  {
    name: 'Risk Hedger',
    address: '0x3C44CdDdB6a900fa2b585dd299e03d12FA4293BC',
    color: 'sage',
    balances: [
      { token: 'ETH', amount: 110, decimals: 4 },
      { token: 'USDC', amount: 0, decimals: 2 },
    ],
    keyMetric: { label: 'HF', value: '--' },
  },
];

// ---------------------------------------------------------------------------
// Hook
// ---------------------------------------------------------------------------

/**
 * Wraps `useChainWs` with derived block data and position updates.
 *
 * Blocks are synthesized from pheromone events (each pheromone deposit is
 * treated as a block confirmation). Positions are updated with computed
 * key metrics when new pheromone data arrives.
 */
export function useChainData(enabled = true): ChainData {
  const ws = useChainWs(enabled);

  // Derive BlockData[] from pheromone events
  const blocks: BlockData[] = useMemo(() => {
    return ws.pheromones.slice(-20).map((p: PheromoneEvent) => ({
      number: p.id,
      timestamp: p.depositedAt,
      transactions: [
        {
          hash: `0x${p.id.toString(16).padStart(8, '0')}${p.kind.slice(0, 4)}`,
          type: (
            p.kind === 'strategy'
              ? 'defi'
              : p.kind === 'causal'
                ? 'insight'
                : 'other'
          ) as TxData['type'],
          description: `${p.kind} pheromone (intensity: ${p.intensity.toFixed(2)})`,
        },
      ],
    }));
  }, [ws.pheromones]);

  // Mutable positions state derived from default + WS updates
  const positions: AgentPosition[] = useMemo(() => {
    const totalPheromones = ws.stats.pheromones;
    if (!ws.connected || totalPheromones === 0) return DEFAULT_POSITIONS;

    return DEFAULT_POSITIONS.map((pos, i) => ({
      ...pos,
      keyMetric:
        i === 0
          ? { label: 'APR', value: `${(5 + totalPheromones * 0.1).toFixed(1)}%` }
          : { label: 'HF', value: `${(2.0 + totalPheromones * 0.02).toFixed(2)}` },
    }));
  }, [ws.connected, ws.stats.pheromones]);

  return { connected: ws.connected, blocks, positions, ws };
}
