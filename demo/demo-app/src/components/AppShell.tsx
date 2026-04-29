import { useEffect } from 'react';
import { Outlet } from 'react-router';
import Grain from './Grain';
import HeroParticleField from './HeroParticleField';
import Curtain from './Curtain';
import ScrollTrack from './ScrollTrack';
import TopNav from './TopNav';
import ConfigWidget from './ConfigWidget';
import { RokoConfigProvider } from '../hooks/useRokoConfig';

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
      <Grain />
      <HeroParticleField />
      <Curtain />
      <ScrollTrack />
      <TopNav />
      <ConfigWidget />
      <div className="app-frame" style={{ paddingTop: 64, position: 'relative', zIndex: 1, minHeight: '100vh' }}>
        <Outlet />
      </div>
    </RokoConfigProvider>
  );
}
