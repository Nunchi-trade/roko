import { lazy, Suspense, useEffect } from 'react';
import { Outlet } from 'react-router';
import Grain from './Grain';
import Curtain from './Curtain';
import ScrollTrack from './ScrollTrack';
import TopNav from './TopNav';
import ComponentErrorBoundary from './design/ComponentErrorBoundary';
import { useApiWithFallback } from '../hooks/useApiWithFallback';

const LazyHeroParticleField = lazy(() => import('./HeroParticleField'));

export default function AppShell() {
  const { dataMode } = useApiWithFallback();

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
      {dataMode === 'seed' && (
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
          SEED DATA
        </div>
      )}
      <div className="app-frame" style={{ paddingTop: 48, position: 'relative', zIndex: 1, minHeight: '100vh' }}>
        <Outlet />
      </div>
    </>
  );
}
