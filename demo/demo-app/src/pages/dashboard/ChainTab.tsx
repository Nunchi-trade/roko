import { useEffect, useCallback } from 'react';
import { useDataHub } from '../../app/DataHub';
import type { ChainBlockEntry, ChainTxEntry, ChainEventEntry } from '../../app/DataHub';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import { useDebouncedRefetch } from '../../hooks/useDebouncedRefetch';
import Oscilloscope from '../../components/canvas/Oscilloscope';
import './ChainTab.css';

export default function ChainTab() {
  const chainBlocks = useDataHub((s) => s.chainBlocks);
  const chainTxs = useDataHub((s) => s.chainTxs);
  const chainEvents = useDataHub((s) => s.chainEvents);
  const chainLatestBlock = useDataHub((s) => s.chainLatestBlock);
  const chainWatcherRunning = useDataHub((s) => s.chainWatcherRunning);
  const chainGasHistory = useDataHub((s) => s.chainGasHistory);
  const fetchChainBlocks = useDataHub((s) => s.fetchChainBlocks);
  const fetchChainTxs = useDataHub((s) => s.fetchChainTxs);
  const fetchChainEvents = useDataHub((s) => s.fetchChainEvents);
  const fetchChainStatus = useDataHub((s) => s.fetchChainStatus);

  // Initial fetch
  useEffect(() => {
    fetchChainBlocks();
    fetchChainTxs();
    fetchChainEvents();
    fetchChainStatus();
  }, [fetchChainBlocks, fetchChainTxs, fetchChainEvents, fetchChainStatus]);

  // SSE-triggered refetch (debounced)
  const fetchAll = useCallback(() => {
    fetchChainBlocks();
    fetchChainTxs();
    fetchChainEvents();
  }, [fetchChainBlocks, fetchChainTxs, fetchChainEvents]);

  const debouncedRefetch = useDebouncedRefetch(fetchAll, 3000);
  useContextEventSubscription(
    ['chain_block', 'chain_tx', 'chain_contract_event'],
    useCallback(() => debouncedRefetch(), [debouncedRefetch]),
  );

  return (
    <div className="chain-grid dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
      {/* Block Feed */}
      <div className="chain-panel">
        <div className="chain-panel__header">
          <span>BLOCK FEED</span>
          <span>
            <span className={`chain-watcher-dot ${chainWatcherRunning ? 'chain-watcher-dot--active' : 'chain-watcher-dot--inactive'}`} />
            {' '}#{chainLatestBlock?.number ?? '—'}
          </span>
        </div>
        <div className="chain-panel__body">
          {chainBlocks.length === 0 ? (
            <div className="chain-empty">Waiting for blocks…</div>
          ) : (
            chainBlocks.map((b) => <BlockRow key={b.number} block={b} />)
          )}
        </div>
      </div>

      {/* Gas Chart */}
      <div className="chain-panel">
        <div className="chain-panel__header">
          <span>GAS UTILIZATION</span>
          <span>{chainGasHistory.length} blocks</span>
        </div>
        <div className="chain-gas-chart">
          <Oscilloscope data={chainGasHistory} height={160} />
        </div>
      </div>

      {/* Transaction Log */}
      <div className="chain-panel">
        <div className="chain-panel__header">
          <span>TRANSACTIONS</span>
          <span>{chainTxs.length} buffered</span>
        </div>
        <div className="chain-panel__body">
          {chainTxs.length === 0 ? (
            <div className="chain-empty">No transactions yet</div>
          ) : (
            chainTxs.map((tx, i) => <TxRow key={`${tx.txHash}-${i}`} tx={tx} />)
          )}
        </div>
      </div>

      {/* Contract Events */}
      <div className="chain-panel">
        <div className="chain-panel__header">
          <span>CONTRACT EVENTS</span>
          <span>{chainEvents.length} decoded</span>
        </div>
        <div className="chain-panel__body">
          {chainEvents.length === 0 ? (
            <div className="chain-empty">No contract events yet</div>
          ) : (
            chainEvents.map((evt, i) => <EventRow key={`${evt.txHash}-${evt.logIndex}-${i}`} event={evt} />)
          )}
        </div>
      </div>
    </div>
  );
}

/* ── Block Row ──────────────────────────────────────────────────── */

function BlockRow({ block }: { block: ChainBlockEntry }) {
  const ago = Math.floor((Date.now() / 1000) - block.timestamp);
  const gasPercent = block.gasLimit > 0 ? (block.gasUsed / block.gasLimit) * 100 : 0;

  return (
    <div className="chain-block-row">
      <span className="chain-block-row__num">#{block.number}</span>
      <span className="chain-block-row__hash">{truncHash(block.hash)}</span>
      <span className="chain-block-row__ago">{ago}s</span>
      <div className="chain-block-row__gas-bar">
        <div className="chain-block-row__gas-fill" style={{ width: `${gasPercent}%` }} />
      </div>
      <span className="chain-block-row__txcount">{block.txCount}tx</span>
    </div>
  );
}

/* ── Transaction Row ────────────────────────────────────────────── */

function TxRow({ tx }: { tx: ChainTxEntry }) {
  const isCreate = tx.to === null;
  const cls = `chain-tx-row${!tx.success ? ' chain-tx-row--fail' : ''}${isCreate ? ' chain-tx-row--create' : ''}`;

  return (
    <div className={cls}>
      <span className="chain-tx-row__block">#{tx.blockNumber}</span>
      <span className="chain-tx-row__addrs">
        {truncAddr(tx.from)} → {isCreate ? 'CREATE' : truncAddr(tx.to!)}
      </span>
      <span className="chain-tx-row__value">{formatWei(tx.valueWei)}</span>
      <span className="chain-tx-row__gas">{formatGas(tx.gasUsed)}</span>
      <span className="chain-tx-row__method">{tx.methodSig ?? '—'}</span>
    </div>
  );
}

/* ── Event Row ──────────────────────────────────────────────────── */

function EventRow({ event }: { event: ChainEventEntry }) {
  const colorClass = getEventColorClass(event.eventName);
  const params = Object.entries(event.decoded)
    .map(([k, v]) => `${k}=${String(v)}`)
    .join(' ');

  return (
    <div className={`chain-event-row ${colorClass}`}>
      <span className="chain-event-row__name">{event.eventName}</span>
      <span className="chain-event-row__params" title={params}>{params}</span>
      <span className="chain-event-row__block">#{event.blockNumber}</span>
    </div>
  );
}

/* ── Helpers ────────────────────────────────────────────────────── */

function truncHash(hash: string): string {
  if (hash.length <= 12) return hash;
  return `${hash.slice(0, 6)}…${hash.slice(-4)}`;
}

function truncAddr(addr: string): string {
  if (addr.length <= 10) return addr;
  return `${addr.slice(0, 6)}…${addr.slice(-4)}`;
}

function formatWei(wei: string): string {
  const n = BigInt(wei);
  if (n === 0n) return '0';
  const eth = Number(n) / 1e18;
  if (eth < 0.001) return '<0.001 ETH';
  return `${eth.toFixed(3)} ETH`;
}

function formatGas(gas: number): string {
  if (gas >= 1_000_000) return `${(gas / 1_000_000).toFixed(1)}M`;
  if (gas >= 1_000) return `${(gas / 1_000).toFixed(0)}k`;
  return String(gas);
}

function getEventColorClass(name: string): string {
  if (name === 'RateSubmitted') return 'chain-event--rate';
  if (name === 'KeeperRewarded') return 'chain-event--reward';
  if (name === 'RoleGranted' || name === 'RoleRevoked') return 'chain-event--role';
  return 'chain-event--unknown';
}
