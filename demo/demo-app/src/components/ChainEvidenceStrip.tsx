import './ChainEvidenceStrip.css';

export interface ChainTx {
  block: number;
  fn: string;
  amount?: string;
  type: 'fund' | 'advance' | 'vote';
}

interface ChainEvidenceStripProps {
  txs: ChainTx[];
}

export default function ChainEvidenceStrip({ txs }: ChainEvidenceStripProps) {
  if (txs.length === 0) {
    return (
      <div className="chain-evidence-strip">
        <span className="chain-evidence-empty">awaiting chain activity</span>
      </div>
    );
  }

  return (
    <div className="chain-evidence-strip">
      {txs.map((tx, i) => (
        <div key={`${tx.block}-${tx.fn}-${i}`} className="chain-tx">
          {i > 0 && <span className="chain-tx-dash">·</span>}
          <span className="block">#{tx.block}</span>
          <span className="fn">{tx.fn}</span>
          {tx.amount && <span className="amt">{tx.amount}</span>}
          <span className={`dot ${tx.type}`} />
        </div>
      ))}
    </div>
  );
}
