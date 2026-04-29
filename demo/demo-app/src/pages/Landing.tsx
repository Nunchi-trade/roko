import { useState, useEffect } from 'react';
import { Link } from 'react-router';
import HeroScene from '../components/HeroScene';
import '../components/HeroScene.css';

const LOOP_PHASES = [
  'query', 'score', 'route', 'compose', 'act', 'verify', 'write', 'react',
] as const;

export default function Landing() {
  const [activeStep, setActiveStep] = useState(0);

  useEffect(() => {
    const id = setInterval(() => {
      setActiveStep((s) => (s + 1) % LOOP_PHASES.length);
    }, 2200);
    return () => clearInterval(id);
  }, []);

  return (
    <div style={{ height: 'calc(100vh - 48px)', overflow: 'hidden', position: 'relative' }}>
      {/* 3D scene — full viewport */}
      <HeroScene activeStep={activeStep} />

      {/* NieR-style corner marks */}
      <div className="nier-corner tl" />
      <div className="nier-corner tr" />
      <div className="nier-corner bl" />
      <div className="nier-corner br" />

      {/* Centered title composition */}
      <div className="landing-center landing-fade-in">
        {/* Ornamental rule */}
        <div className="nier-rule" />

        {/* Title */}
        <h1 className="landing-title">
          <span className="accent">nunchi</span>
        </h1>

        {/* Subtitle */}
        <p className="landing-sub">the agent coordination plane</p>

        {/* Lower rule */}
        <div className="nier-rule-lower" />

        {/* START button */}
        <Link to="/demo" className="start-btn">
          <span className="start-corner tl" />
          <span className="start-corner tr" />
          <span className="start-corner bl" />
          <span className="start-corner br" />
          start
        </Link>

        {/* Loop phase ticker */}
        <div className="landing-loop-ticker">
          {LOOP_PHASES.map((phase, i) => (
            <span key={phase}>
              {i > 0 && <span className="sep" />}
              <span className={`phase ${i === activeStep ? 'active' : ''}`}>
                {phase}
              </span>
            </span>
          ))}
        </div>
      </div>

      {/* Bottom center mark */}
      <div className="landing-footer-mark">
        18 crates &middot; one universal loop
      </div>
    </div>
  );
}
