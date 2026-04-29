import { lazy, Suspense, useEffect, useRef } from 'react';
import { Outlet } from 'react-router';
import Grain from './Grain';
import Curtain from './Curtain';
import ScrollTrack from './ScrollTrack';
import TopNav from './TopNav';
import { ArtifactTray } from './inference';
import ComponentErrorBoundary from './design/ComponentErrorBoundary';
import { useDataHub } from '../app/DataHub';

const LazyHeroParticleField = lazy(() => import('./HeroParticleField'));

type ArtifactType = 'episode' | 'insight' | 'hdc' | 'knowledge';

export default function AppShell() {
  const serverStatus = useDataHub((s) => s.serverStatus);

  // T7.56: Derive artifact counts from DataHub state
  const episodeCount = useDataHub((s) => s.episodes.length);
  const inferenceCount = useDataHub((s) => s.recentInferences.length);
  // HDC and knowledge are not yet tracked in DataHub; default to 0
  const hdcCount = 0;
  const knowledgeCount = 0;

  // Determine most-recent artifact type for pop animation
  const prevEpisodes = useRef(0);
  const prevInferences = useRef(0);
  let recentType: ArtifactType | null = null;
  if (episodeCount > prevEpisodes.current) recentType = 'episode';
  else if (inferenceCount > prevInferences.current) recentType = 'insight';
  prevEpisodes.current = episodeCount;
  prevInferences.current = inferenceCount;

  useEffect(() => {
    const io = new IntersectionObserver(
      (entries) => {
        entries.forEach((e) => {
          if (e.isIntersecting) {
            e.target.classList.add('in');
            io.unobserve(e.target);
          }
        });
      },
      { threshold: 0.18 },
    );
    document.querySelectorAll('.reveal').forEach((el) => io.observe(el));
    return () => io.disconnect();
  }, []);

  return (
    <>
      <Grain />
      <ComponentErrorBoundary name="HeroParticleField">
        <Suspense fallback={null}>
          <LazyHeroParticleField />
        </Suspense>
      </ComponentErrorBoundary>
      <Curtain />
      <ScrollTrack />
      <TopNav />
      {/* T7.56: Persistent artifact tray in app chrome */}
      {(episodeCount + inferenceCount + hdcCount + knowledgeCount) > 0 && (
        <div
          style={{
            position: 'fixed',
            top: 52,
            left: '50%',
            transform: 'translateX(-50%)',
            zIndex: 9000,
            pointerEvents: 'auto',
          }}
        >
          <ArtifactTray
            episodes={episodeCount}
            insights={inferenceCount}
            hdcEntries={hdcCount}
            knowledgeEntries={knowledgeCount}
            recentType={recentType}
            compact
          />
        </div>
      )}
      {serverStatus === 'disconnected' && (
        <div
          style={{
            position: 'fixed',
            top: 52,
            right: 12,
            zIndex: 9000,
            background: 'rgba(255, 200, 0, 0.15)',
            border: '1px solid rgba(255, 200, 0, 0.4)',
            borderRadius: 4,
            padding: '2px 8px',
            fontSize: 11,
            fontFamily: 'monospace',
            color: 'rgba(255, 200, 0, 0.9)',
            letterSpacing: '0.08em',
            pointerEvents: 'none',
            userSelect: 'none',
          }}
        >
          OFFLINE
        </div>
      )}
      <div className="app-frame" style={{ paddingTop: 48, position: 'relative', zIndex: 1, minHeight: '100vh' }}>
        <Outlet />
      </div>
    </>
  );
}
