import { Suspense, lazy, useEffect, useMemo, useRef } from 'react';
import { Outlet, useNavigate, useLocation } from 'react-router';
import Grain from './Grain';
import Curtain from './Curtain';
import ScrollTrack from './ScrollTrack';
import TopNav from './TopNav';
import HelpOverlay from './HelpOverlay';
import ComponentErrorBoundary from './design/ComponentErrorBoundary';
import { useKeyboardShortcuts, useHelpOverlay, type ShortcutDef } from '../hooks/useKeyboardShortcuts';

const LazyHeroParticleField = lazy(() => import('./HeroParticleField'));
const LazyDemo = lazy(() => import('../pages/Demo/index'));

function RouteLoading() {
  return (
    <div className="route-loading progressive-reveal">
      <div className="route-loading__nav">
        <div className="skeleton route-loading__nav-pill" />
        <div className="skeleton route-loading__nav-pill" style={{ width: 96 }} />
      </div>
      <div className="route-loading__header">
        <div className="skeleton skeleton-circle" />
        <div className="skeleton skeleton-title" />
      </div>
    </div>
  );
}

export default function AppShell() {
  const navigate = useNavigate();
  const location = useLocation();
  const help = useHelpOverlay();

  // KeepAlive: once Demo is visited, keep it mounted (display:none when elsewhere)
  const isDemo = location.pathname === '/demo';
  const demoVisitedRef = useRef(false);
  if (isDemo) demoVisitedRef.current = true;

  // Global keyboard shortcuts (E4/E5)
  const shortcuts = useMemo<ShortcutDef[]>(() => [
    { keys: '?', description: 'Show keyboard shortcuts', category: 'General', action: help.toggle },
    { keys: 'Ctrl+/', description: 'Show keyboard shortcuts', category: 'General', action: help.toggle },
    { keys: 'Escape', description: 'Close overlay / modal', category: 'General', action: help.close },
    { keys: 'g d', description: 'Go to Dashboard', category: 'Navigation', action: () => navigate('/dashboard') },
    { keys: 'g t', description: 'Go to Terminal', category: 'Navigation', action: () => navigate('/terminal') },
    { keys: 'g b', description: 'Go to Bench', category: 'Navigation', action: () => navigate('/bench') },
    { keys: 'g e', description: 'Go to Explorer', category: 'Navigation', action: () => navigate('/explorer') },
    { keys: 'g m', description: 'Go to Demo', category: 'Navigation', action: () => navigate('/demo') },
    { keys: 'g s', description: 'Go to Settings', category: 'Navigation', action: () => navigate('/settings') },
    { keys: 'g p', description: 'Go to Builder', category: 'Navigation', action: () => navigate('/builder') },
  ], [help.toggle, help.close, navigate]);

  useKeyboardShortcuts(shortcuts);

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
      <main id="main-content" role="main" aria-label="Page content" className="app-frame" style={{ paddingTop: 48, position: 'relative', zIndex: 1, minHeight: '100vh' }}>
        {demoVisitedRef.current && (
          <div style={{ display: isDemo ? 'contents' : 'none' }}>
            <Suspense fallback={<RouteLoading />}><LazyDemo /></Suspense>
          </div>
        )}
        <div style={{ display: isDemo ? 'none' : 'contents' }}>
          <Outlet />
        </div>
      </main>
      <HelpOverlay open={help.open} onClose={help.close} shortcuts={shortcuts} />
    </>
  );
}
