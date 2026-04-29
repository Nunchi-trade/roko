import { lazy, Suspense, useEffect, useRef, useState } from 'react';
import { Outlet, useLocation } from 'react-router';
import Grain from './Grain';
import Curtain from './Curtain';
import ScrollTrack from './ScrollTrack';
import TopNav from './TopNav';
import ConfigWidget from './ConfigWidget';
import { ArtifactTray } from './inference';
import ComponentErrorBoundary from './design/ComponentErrorBoundary';
import { useDataHub } from '../app/DataHub';
import './PageTransition.css';

const LazyHeroParticleField = lazy(() => import('./HeroParticleField'));

type ArtifactType = 'episode' | 'insight' | 'hdc' | 'knowledge';

export default function AppShell() {
  const serverStatus = useDataHub((s) => s.serverStatus);
  const location = useLocation();

  // Page transition state
  const [transitionClass, setTransitionClass] = useState('page-enter-active');
  const prevPathRef = useRef(location.pathname);

  useEffect(() => {
    if (location.pathname === prevPathRef.current) return;
    prevPathRef.current = location.pathname;

    // Start exit animation
    setTransitionClass('page-exit-active');

    const exitTimer = window.setTimeout(() => {
      // After exit completes, snap to enter start position then animate in
      setTransitionClass('page-enter');
      // Use rAF to ensure the browser paints the enter start state before transitioning
      requestAnimationFrame(() => {
        requestAnimationFrame(() => {
          setTransitionClass('page-enter-active');
        });
      });
    }, 100); // matches exit transition duration

    return () => window.clearTimeout(exitTimer);
  }, [location.pathname]);

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
      <a href="#main-content" className="skip-to-content">
        Skip to content
      </a>
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
      {/* Floating config pill — always accessible */}
      <div style={{ position: 'fixed', bottom: 16, right: 16, zIndex: 9000 }}>
        <ConfigWidget />
      </div>
      {/* Ambient background: aurora gradients + grain + vignette */}
      <div className="ambient-aurora" aria-hidden="true" />
      <div className="ambient-grain" aria-hidden="true" />
      <div className="ambient-vignette" aria-hidden="true" />
      <div id="main-content" className="app-frame" style={{ paddingTop: 48, position: 'relative', zIndex: 1, flex: 1 }}>
        <div className={`page-transition ${transitionClass}`}>
          <Outlet />
        </div>
      </div>
    </>
  );
}
