import type { Scenario } from '../lib/scenarios';
import type { ServerStatus } from '../hooks/useServerHealth';
import './ScenarioPreview.css';

interface ScenarioPreviewProps {
  scenario: Scenario;
  onPlay: () => void;
  serverHealth: ServerStatus;
  isRunning: boolean;
  dismissing?: boolean;
}

/** Render the flow diagram for prd-research-loop. */
function FlowDiagram() {
  const nodes = ['Idea', 'Draft', 'Research', 'Plan', 'Execute', 'Gates', 'Learn'];
  return (
    <div className="sp-flow">
      {nodes.map((n, i) => (
        <span key={n}>
          <span className="sp-flow-node">{n}</span>
          {i < nodes.length - 1 && <span className="sp-flow-arrow">{' \u2192 '}</span>}
        </span>
      ))}
    </div>
  );
}

/** Two-lane race metaphor. */
function RaceLanes({ left, right }: { left: string; right: string }) {
  return (
    <div className="sp-race-lanes">
      <div className="sp-race-lane">
        <span className="sp-race-lane-label">{left}</span>
        <div className="sp-race-track" />
      </div>
      <div className="sp-race-lane">
        <span className="sp-race-lane-label">{right}</span>
        <div className="sp-race-track" />
      </div>
    </div>
  );
}

/** Gate with retry loop. */
function RetryLoop() {
  return (
    <div className="sp-retry-loop">
      <span className="sp-retry-icon">{'\u21BB'}</span>
      <span className="sp-retry-label">Gate fail {'\u2192'} classify {'\u2192'} replan {'\u2192'} retry</span>
    </div>
  );
}

/** Grid of provider chips. */
function ProviderGrid({ providers }: { providers: string[] }) {
  return (
    <div className="sp-provider-grid">
      {providers.map((p) => (
        <span key={p} className="sp-provider-chip">{p}</span>
      ))}
    </div>
  );
}

/** 2x2 category grid. */
function CategoryGrid({ items }: { items: string[] }) {
  return (
    <div className="sp-category-grid">
      {items.map((c) => (
        <span key={c} className="sp-category-cell">{c}</span>
      ))}
    </div>
  );
}

/** Simple icon + label diagram. */
function IconDiagram({ glyph, label }: { glyph: string; label: string }) {
  return (
    <div className="sp-icon-diagram">
      <span className="sp-icon-glyph">{glyph}</span>
      <span className="sp-icon-label">{label}</span>
    </div>
  );
}

/** Two nodes with bidirectional arrows. */
function TransferDiagram({ left, right }: { left: string; right: string }) {
  return (
    <div className="sp-transfer">
      <span className="sp-transfer-node">{left}</span>
      <span className="sp-transfer-arrows">
        <span>{'\u2192'}</span>
        <span>{'\u2190'}</span>
      </span>
      <span className="sp-transfer-node">{right}</span>
    </div>
  );
}

/** Chain blocks linked together. */
function ChainBlocks() {
  const blocks = ['\u25A3', '\u25A3', '\u25A3', '\u25A3'];
  return (
    <div className="sp-chain-blocks">
      {blocks.map((b, i) => (
        <span key={i}>
          <span className="sp-chain-block">{b}</span>
          {i < blocks.length - 1 && <span className="sp-chain-link" />}
        </span>
      ))}
    </div>
  );
}

/** Scenario-specific visual diagram. */
function ScenarioDiagram({ scenarioId }: { scenarioId: string }) {
  switch (scenarioId) {
    case 'prd-research-loop':
      return <FlowDiagram />;
    case 'race':
      return <RaceLanes left="Naive" right="Cascade" />;
    case 'gate-retry':
      return <RetryLoop />;
    case 'providers':
      return <ProviderGrid providers={['Zhipu', 'OpenAI', 'Anthropic', 'Moonshot']} />;
    case 'provider-race':
      return <RaceLanes left="Anthropic" right="Gemini" />;
    case 'explore':
      return <CategoryGrid items={['Config', 'Knowledge', 'Learning', 'Workspace']} />;
    case 'knowledge-accumulation':
      return <IconDiagram glyph={'\u2593\u2592\u2591'} label="Growing knowledge store" />;
    case 'dream-consolidation':
      return <IconDiagram glyph={'\u263D'} label="Episodes distilled into knowledge" />;
    case 'chat':
      return <IconDiagram glyph={'\u2709'} label="Interactive agent conversation" />;
    case 'knowledge-transfer':
      return <TransferDiagram left="Agent Alpha" right="Agent Beta" />;
    case 'chain-intelligence':
      return <ChainBlocks />;
    case 'mirage':
      return <IconDiagram glyph={'\u26D3'} label="Fork any EVM chain" />;
    default:
      return <IconDiagram glyph={'\u25B6'} label={scenarioId} />;
  }
}

/** What this scenario demonstrates. */
function scenarioFeature(scenarioId: string): string {
  switch (scenarioId) {
    case 'prd-research-loop': return 'Full self-hosting loop';
    case 'race': return 'Cost comparison';
    case 'gate-retry': return 'Gate failure recovery';
    case 'providers': return 'Provider-agnostic dispatch';
    case 'provider-race': return 'First-to-pass-gates wins';
    case 'explore': return '18 crates, 85 routes';
    case 'knowledge-accumulation': return 'Persistent learning';
    case 'dream-consolidation': return 'Offline consolidation';
    case 'chat': return 'Interactive agent chat';
    case 'knowledge-transfer': return 'Cross-agent learning';
    case 'chain-intelligence': return 'On-chain knowledge graph';
    case 'mirage': return 'Local EVM fork';
    default: return scenarioId;
  }
}

export default function ScenarioPreview({
  scenario,
  onPlay,
  serverHealth,
  isRunning,
  dismissing,
}: ScenarioPreviewProps) {
  const healthLabel =
    serverHealth === 'connected'
      ? 'serve live'
      : serverHealth === 'checking'
        ? 'checking'
        : 'serve offline';

  return (
    <div
      className={`sp-overlay${dismissing ? ' dismissing' : ''}`}
      onClick={onPlay}
    >
      {/* Left: terminal layout preview */}
      <div className="sp-terminals" onClick={(e) => e.stopPropagation()}>
        <div className={`sp-term-grid sp-grid-${scenario.panes}`}>
          {Array.from({ length: scenario.panes }).map((_, i) => (
            <div className="sp-mini-pane" key={i}>
              <div className="sp-mini-head">
                <span className="sp-mini-dot" />
                <span className="sp-mini-label">{scenario.labels[i] || `pane ${i + 1}`}</span>
              </div>
              <div className="sp-mini-body">
                <span className="sp-cursor" />
              </div>
            </div>
          ))}
        </div>
      </div>

      {/* Right: scenario info card */}
      <div className="sp-info" onClick={(e) => e.stopPropagation()}>
        <div className="sp-title">{scenario.title}</div>
        <div className="sp-subtitle">{scenario.subtitle}</div>

        <div className="sp-facts">
          <span className="sp-fact">
            <span className="sp-fact-icon">{'\u25A0'}</span>
            {scenario.panes} terminal{scenario.panes > 1 ? 's' : ''}
          </span>
          <span className="sp-fact">
            <span className="sp-fact-icon">{'\u25C6'}</span>
            {scenarioFeature(scenario.id)}
          </span>
        </div>

        <div className="sp-diagram">
          <ScenarioDiagram scenarioId={scenario.id} />
        </div>

        <button
          className="sp-start-btn"
          disabled={isRunning}
          onClick={(e) => {
            e.stopPropagation();
            onPlay();
          }}
        >
          Start
        </button>

        <div className={`sp-health ${serverHealth}`}>
          <span className="sp-health-dot" />
          {healthLabel}
        </div>
      </div>
    </div>
  );
}
