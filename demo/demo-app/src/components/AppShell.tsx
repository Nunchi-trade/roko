import { useEffect } from 'react';
import { Outlet } from 'react-router';
import Grain from './Grain';
import HeroParticleField from './HeroParticleField';
import Curtain from './Curtain';
import ScrollTrack from './ScrollTrack';
import TopNav from './TopNav';
import { useSeedData } from '../hooks/useSeedData';

export default function AppShell() {
  useSeedData();

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
      <HeroParticleField />
      <Curtain />
      <ScrollTrack />
      <TopNav />
      <div className="app-frame" style={{ paddingTop: 48, position: 'relative', zIndex: 1, minHeight: '100vh' }}>
        <Outlet />
      </div>
    </>
  );
}
