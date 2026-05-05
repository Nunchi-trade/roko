import './ProvenanceCard.css';

interface ProvenanceCardProps {
  model: string;
  runId?: string;
  timestamp?: string;
  cost?: string;
  tokens?: string;
  duration?: string;
}

export default function ProvenanceCard({
  model,
  runId,
  timestamp,
  cost,
  tokens,
  duration,
}: ProvenanceCardProps) {
  const now = timestamp || new Date().toLocaleTimeString();
  const shortId = runId ? runId.slice(0, 12) : null;

  return (
    <section className="provenance-card" aria-label="Run provenance">
      <div className="provenance-header">
        <span className="provenance-title">Provenance</span>
        <span className="provenance-ts">{now}</span>
      </div>
      <div className="provenance-rows">
        <div className="provenance-row">
          <span className="provenance-label">model</span>
          <span className="provenance-value provenance-value--model">{model || '--'}</span>
        </div>
        {shortId && (
          <div className="provenance-row">
            <span className="provenance-label">run</span>
            <span className="provenance-value provenance-value--id">{shortId}</span>
          </div>
        )}
        {cost && (
          <div className="provenance-row">
            <span className="provenance-label">cost</span>
            <span className="provenance-value">{cost}</span>
          </div>
        )}
        {tokens && (
          <div className="provenance-row">
            <span className="provenance-label">tokens</span>
            <span className="provenance-value">{tokens}</span>
          </div>
        )}
        {duration && (
          <div className="provenance-row">
            <span className="provenance-label">duration</span>
            <span className="provenance-value">{duration}</span>
          </div>
        )}
      </div>
    </section>
  );
}
