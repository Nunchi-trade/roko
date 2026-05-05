import { useEffect, useCallback, useRef, useState } from 'react';
import { useDataHub } from '../../app/DataHub';
import type { ChainBlockEntry, ChainTxEntry, ChainEventEntry } from '../../app/DataHub';
import { useContextEventSubscription } from '../../contexts/EventStreamContext';
import Oscilloscope from '../../components/canvas/Oscilloscope';
import './ChainTab.css';

export default function ChainTab() {
  const chainBlocks = useDataHub((s) => s.chainBlocks);
  const chainTxs = useDataHub((s) => s.chainTxs);
  const chainEvents = useDataHub((s) => s.chainEvents);
  const chainLatestBlock = useDataHub((s) => s.chainLatestBlock);
  const chainGasHistory = useDataHub((s) => s.chainGasHistory);
  const fetchChainBlocks = useDataHub((s) => s.fetchChainBlocks);
  const fetchChainTxs = useDataHub((s) => s.fetchChainTxs);
  const fetchChainEvents = useDataHub((s) => s.fetchChainEvents);
  const fetchChainStatus = useDataHub((s) => s.fetchChainStatus);

  const [selectedBlock, setSelectedBlock] = useState<number | null>(null);
  const [sseActive, setSseActive] = useState(false);
  const sseTimerRef = useRef<ReturnType<typeof setTimeout> | undefined>(undefined);

  // Auto-select latest block when none selected and new blocks arrive
  useEffect(() => {
    if (selectedBlock === null && chainBlocks.length > 0) {
      setSelectedBlock(chainBlocks[0].number);
    }
  }, [chainBlocks, selectedBlock]);

  // Seed from REST once on mount
  useEffect(() => {
    fetchChainBlocks();
    fetchChainTxs();
    fetchChainEvents();
    fetchChainStatus();
  }, [fetchChainBlocks, fetchChainTxs, fetchChainEvents, fetchChainStatus]);

  // SSE activity tracking
  useContextEventSubscription(
    ['chain_block', 'chain_tx', 'chain_contract_event'],
    useCallback(() => {
      setSseActive(true);
      clearTimeout(sseTimerRef.current);
      sseTimerRef.current = setTimeout(() => setSseActive(false), 10_000);
    }, []),
  );

  useEffect(() => () => clearTimeout(sseTimerRef.current), []);

  // Derived data
  const selected = chainBlocks.find((b) => b.number === selectedBlock) ?? null;
  const filteredTxs = selectedBlock != null
    ? chainTxs.filter((tx) => tx.blockNumber === selectedBlock)
    : chainTxs;
  const filteredEvents = selectedBlock != null
    ? chainEvents.filter((evt) => evt.blockNumber === selectedBlock)
    : chainEvents;

  // Gas strip values from latest block
  const gasPercent = selected
    ? selected.gasLimit > 0 ? (selected.gasUsed / selected.gasLimit) * 100 : 0
    : 0;
  const baseFeeGwei = selected?.baseFeePerGas
    ? (Number(selected.baseFeePerGas) / 1e9).toFixed(1)
    : '—';

  return (
    <div className="chain-explorer progressive-reveal dash-stagger" style={{ '--stagger-i': 2 } as React.CSSProperties}>
      {/* Explorer Header */}
      <div className="chain-explorer-header">
        <div className="chain-explorer-header__left">
          {sseActive && (
            <span className="chain-sse-live">
              <span className="chain-sse-live__dot" />
              LIVE
            </span>
          )}
          <span className="chain-explorer-header__label">Block Explorer</span>
        </div>
        <div className="chain-explorer-header__right">
          <span className="chain-explorer-header__block-num">
            #{chainLatestBlock?.number ?? '---'}
          </span>
          {chainLatestBlock && (
            <span>{Math.floor(Date.now() / 1000 - chainLatestBlock.timestamp)}s</span>
          )}
          {chainLatestBlock && (
            <span>{'\u2B21'} {formatGas(chainLatestBlock.gasUsed)} gas</span>
          )}
          <span>{chainBlocks.length} blocks</span>
        </div>
      </div>

      {/* Gas Strip */}
      <div className="chain-gas-strip">
        <div className="chain-gas-strip__bar">
          <div
            className={`chain-gas-strip__fill ${gasPercent < 50 ? 'chain-gas-strip__fill--low' : gasPercent < 80 ? 'chain-gas-strip__fill--mid' : 'chain-gas-strip__fill--high'}`}
            style={{ width: `${gasPercent}%` }}
          />
        </div>
        <div className="chain-gas-strip__labels">
          <span>Gas: {gasPercent.toFixed(1)}%</span>
          <span>Base: {baseFeeGwei} gwei</span>
        </div>
      </div>

      {/* Block List + Detail split */}
      <div className="chain-split">
        <BlockList
          blocks={chainBlocks}
          selectedBlock={selectedBlock}
          onSelect={setSelectedBlock}
        />
        <BlockDetail block={selected} />
      </div>

      {/* Transactions */}
      <div className="chain-section">
        <div className="chain-section__header">
          <span>TRANSACTIONS ({filteredTxs.length})</span>
          {selectedBlock != null && (
            <span>Block #{selectedBlock}</span>
          )}
        </div>
        <div className="chain-section__body">
          {filteredTxs.length === 0 ? (
            <div className="chain-empty">
              {chainBlocks.length > 0 ? 'No transactions in selected block' : 'Waiting for blocks\u2026'}
            </div>
          ) : (
            filteredTxs.map((tx, i) => <TxRow key={`${tx.txHash}-${i}`} tx={tx} />)
          )}
        </div>
      </div>

      {/* Contract Events */}
      <div className="chain-section">
        <div className="chain-section__header">
          <span>CONTRACT EVENTS ({filteredEvents.length} decoded)</span>
        </div>
        <div className="chain-section__body">
          {filteredEvents.length === 0 ? (
            <div className="chain-empty">
              {chainBlocks.length > 0 ? 'No events in selected block' : 'Waiting for blocks\u2026'}
            </div>
          ) : (
            filteredEvents.map((evt, i) => (
              <EventRow key={`${evt.txHash}-${evt.logIndex}-${i}`} event={evt} />
            ))
          )}
        </div>
      </div>

      {/* Gas Chart */}
      <div className="chain-gas-chart">
        <div className="chain-gas-chart__header">GAS UTILIZATION</div>
        <Oscilloscope data={chainGasHistory} height={120} />
      </div>
    </div>
  );
}

/* ── Block List ────────────────────────────────────────────────── */

function BlockList({
  blocks,
  selectedBlock,
  onSelect,
}: {
  blocks: ChainBlockEntry[];
  selectedBlock: number | null;
  onSelect: (n: number) => void;
}) {
  return (
    <div className="chain-block-list">
      <div className="chain-block-list__header">BLOCKS</div>
      {blocks.length === 0 ? (
        <div className="chain-empty">Waiting for blocks\u2026</div>
      ) : (
        blocks.map((b) => {
          const ago = Math.floor(Date.now() / 1000 - b.timestamp);
          const isSelected = b.number === selectedBlock;
          return (
            <div
              key={b.number}
              className={`chain-block-list__item${isSelected ? ' chain-block-list__item--selected' : ''}`}
              onClick={() => onSelect(b.number)}
            >
              <span className="chain-block-list__item-num">#{b.number}</span>
              <span className="chain-block-list__item-ago">{ago}s</span>
              <span className="chain-block-list__item-txcount">{b.txCount}tx</span>
            </div>
          );
        })
      )}
    </div>
  );
}

/* ── Block Detail ──────────────────────────────────────────────── */

function BlockDetail({ block }: { block: ChainBlockEntry | null }) {
  if (!block) {
    return (
      <div className="chain-detail">
        <div className="chain-detail__empty">Select a block</div>
      </div>
    );
  }

  const gasPercent = block.gasLimit > 0
    ? ((block.gasUsed / block.gasLimit) * 100).toFixed(1)
    : '0';
  const baseFeeGwei = block.baseFeePerGas
    ? (Number(block.baseFeePerGas) / 1e9).toFixed(2)
    : '—';
  const ts = new Date(block.timestamp * 1000).toISOString().replace('T', ' ').slice(0, 19);

  return (
    <div className="chain-detail">
      <div className="chain-detail__header">BLOCK #{block.number}</div>
      <div className="chain-detail__row">
        <span className="chain-detail__label">Hash</span>
        <span className="chain-detail__value chain-detail__value--mono">{block.hash}</span>
      </div>
      <div className="chain-detail__row">
        <span className="chain-detail__label">Parent</span>
        <span className="chain-detail__value chain-detail__value--mono">{block.parentHash}</span>
      </div>
      <div className="chain-detail__row">
        <span className="chain-detail__label">Time</span>
        <span className="chain-detail__value">{ts}</span>
      </div>
      <div className="chain-detail__row">
        <span className="chain-detail__label">Gas</span>
        <span className="chain-detail__value">
          <span className="chain-detail__gas-bar">
            {fmtNum(block.gasUsed)} / {fmtNum(block.gasLimit)} ({gasPercent}%)
            <span className="chain-detail__gas-bar-track">
              <span
                className="chain-detail__gas-bar-fill"
                style={{ width: `${gasPercent}%` }}
              />
            </span>
          </span>
        </span>
      </div>
      <div className="chain-detail__row">
        <span className="chain-detail__label">Base Fee</span>
        <span className="chain-detail__value">{baseFeeGwei} gwei</span>
      </div>
      <div className="chain-detail__row">
        <span className="chain-detail__label">Txns</span>
        <span className="chain-detail__value">{block.txCount}</span>
      </div>
    </div>
  );
}

/* ── Transaction Row ───────────────────────────────────────────── */

function TxRow({ tx }: { tx: ChainTxEntry }) {
  const isCreate = tx.to === null;
  const cls = `chain-tx-row${!tx.success ? ' chain-tx-row--fail' : ''}${isCreate ? ' chain-tx-row--create' : ''}`;

  return (
    <div className={cls}>
      <span className="chain-tx-row__hash">{truncHash(tx.txHash)}</span>
      <span className="chain-tx-row__addrs">
        {truncAddr(tx.from)} {'\u2192'} {isCreate ? 'CREATE' : truncAddr(tx.to!)}
      </span>
      <span className="chain-tx-row__value">{formatWei(tx.valueWei)}</span>
      <span className="chain-tx-row__gas">{formatGas(tx.gasUsed)}</span>
      <span className="chain-tx-row__method">{tx.methodSig ?? '\u2014'}</span>
    </div>
  );
}

/* ── Event Row ─────────────────────────────────────────────────── */

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
      <span className="chain-event-row__tx">{truncHash(event.txHash)}</span>
    </div>
  );
}

/* ── Helpers ───────────────────────────────────────────────────── */

function truncHash(hash: string): string {
  if (hash.length <= 12) return hash;
  return `${hash.slice(0, 6)}\u2026${hash.slice(-4)}`;
}

function truncAddr(addr: string): string {
  if (addr.length <= 10) return addr;
  return `${addr.slice(0, 6)}\u2026${addr.slice(-4)}`;
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

function fmtNum(n: number): string {
  return n.toLocaleString();
}

function getEventColorClass(name: string): string {
  if (name === 'RateSubmitted') return 'chain-event--rate';
  if (name === 'KeeperRewarded') return 'chain-event--reward';
  if (name === 'RoleGranted' || name === 'RoleRevoked') return 'chain-event--role';
  return 'chain-event--unknown';
}
