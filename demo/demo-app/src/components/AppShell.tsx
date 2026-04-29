import { Suspense, lazy, useEffect } from 'react';
import { Outlet } from 'react-router';
import Grain from './Grain';
import Curtain from './Curtain';
import ScrollTrack from './ScrollTrack';
import TopNav from './TopNav';
import ConfigWidget from './ConfigWidget';
import { RokoConfigProvider } from '../hooks/useRokoConfig';
import { WorkspaceProvider } from '../hooks/useWorkspace';

const HeroParticleField = lazy(() => import('./HeroParticleField'));

export default function AppShell() {
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
    <RokoConfigProvider>
      <WorkspaceProvider>
        <Grain />
        <Suspense fallback={null}>
          <HeroParticleField />
        </Suspense>
        <Curtain />
        <ScrollTrack />
        <TopNav />
        <ConfigWidget />
        <div className="app-frame" style={{
          paddingTop: 'var(--nav-h)',
          position: 'relative',
          zIndex: 1,
          height: '100vh',
          display: 'flex',
          flexDirection: 'column',
          overflow: 'hidden',
        }}>
          <div style={{ flex: 1, minHeight: 0, display: 'flex', flexDirection: 'column', overflow: 'hidden' }}>
            <Outlet />
          </div>
        </div>
      </WorkspaceProvider>
    </RokoConfigProvider>
  );
}
