import { useEffect, useRef, useState } from 'react';
import './ArtifactTray.css';

type ArtifactType = 'episode' | 'insight' | 'hdc' | 'knowledge';

interface ArtifactTrayProps {
  episodes: number;
  insights: number;
  hdcEntries: number;
  knowledgeEntries: number;
  recentType?: ArtifactType | null;
  onOpen?: () => void;
  compact?: boolean;
  className?: string;
}

const ARTIFACT_SLOTS: {
  key: ArtifactType;
  icon: string;
  label: string;
  tooltipSingular: string;
  tooltipPlural: string;
}[] = [
  { key: 'episode',   icon: '\u2B21', label: 'EP.',   tooltipSingular: 'Episode',   tooltipPlural: 'Episodes' },
  { key: 'insight',   icon: '\u25C6', label: 'INS.',  tooltipSingular: 'Insight',   tooltipPlural: 'Insights' },
  { key: 'hdc',       icon: '\u25CF', label: 'HDC',   tooltipSingular: 'HDC Entry', tooltipPlural: 'HDC Entries' },
  { key: 'knowledge', icon: '\u25A0', label: 'KNOW.', tooltipSingular: 'Knowledge Entry', tooltipPlural: 'Knowledge Entries' },
];

function countFor(key: ArtifactType, props: ArtifactTrayProps): number {
  switch (key) {
    case 'episode':   return props.episodes;
    case 'insight':   return props.insights;
    case 'hdc':       return props.hdcEntries;
    case 'knowledge': return props.knowledgeEntries;
  }
}

/** Duration (ms) to show the pop + sparkle effect. */
const POP_DURATION = 500;

export default function ArtifactTray(props: ArtifactTrayProps) {
  const {
    recentType = null,
    onOpen,
    compact = false,
    className,
  } = props;

  const prevRecentRef = useRef<ArtifactType | null>(null);
  const [animatingType, setAnimatingType] = useState<ArtifactType | null>(null);
  const timerRef = useRef<number>(0);

  // Detect when recentType changes to a new non-null value to trigger animation.
  useEffect(() => {
    if (recentType && recentType !== prevRecentRef.current) {
      // Clear any in-progress animation timer.
      if (timerRef.current) window.clearTimeout(timerRef.current);

      setAnimatingType(recentType);
      timerRef.current = window.setTimeout(() => {
        setAnimatingType(null);
        timerRef.current = 0;
      }, POP_DURATION);
    }
    prevRecentRef.current = recentType ?? null;
  }, [recentType]);

  // Cleanup on unmount.
  useEffect(() => {
    return () => {
      if (timerRef.current) window.clearTimeout(timerRef.current);
    };
  }, []);

  const rootClasses = [
    'artifact-tray',
    compact && 'artifact-tray--compact',
    onOpen && 'artifact-tray--clickable',
    className,
  ]
    .filter(Boolean)
    .join(' ');

  const sizeClass = compact ? 'artifact-tray__shape--compact' : 'artifact-tray__shape--normal';

  return (
    <div
      className={rootClasses}
      onClick={onOpen}
      role={onOpen ? 'button' : undefined}
      tabIndex={onOpen ? 0 : undefined}
      onKeyDown={onOpen ? (e) => { if (e.key === 'Enter' || e.key === ' ') onOpen(); } : undefined}
    >
      {ARTIFACT_SLOTS.map(({ key, label, tooltipSingular, tooltipPlural }) => {
        const count = countFor(key, props);
        const isAnimating = animatingType === key;
        const tooltip = `${count} ${count === 1 ? tooltipSingular : tooltipPlural}`;

        return (
          <div
            key={key}
            className={`artifact-tray__slot${isAnimating ? ' artifact-tray__slot--pop' : ''}`}
            data-tooltip={tooltip}
          >
            <span className="artifact-tray__counter">
              <span
                className={`artifact-tray__shape ${sizeClass} artifact-tray__shape--${key}`}
                aria-hidden="true"
              />
              <span className={`artifact-tray__count artifact-tray__count--${key}`}>
                {count}
              </span>
            </span>

            {!compact && (
              <span className="artifact-tray__label">{label}</span>
            )}

            {/* Sparkle particles on new artifact */}
            {isAnimating && (
              <>
                <span className={`artifact-tray__sparkle artifact-tray__sparkle--1 artifact-tray__sparkle--${key}`} />
                <span className={`artifact-tray__sparkle artifact-tray__sparkle--2 artifact-tray__sparkle--${key}`} />
                <span className={`artifact-tray__sparkle artifact-tray__sparkle--3 artifact-tray__sparkle--${key}`} />
              </>
            )}
          </div>
        );
      })}

      {!compact && onOpen && (
        <button
          type="button"
          className="artifact-tray__view-all"
          onClick={(e) => {
            e.stopPropagation();
            onOpen();
          }}
        >
          VIEW ALL
        </button>
      )}
    </div>
  );
}
