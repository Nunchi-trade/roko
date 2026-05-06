import { useState, useEffect } from 'react';
import { Link } from 'react-router';
import AgentSwarm from '../components/three/AgentSwarm';
import './Landing.css';

const LOOP_PHASES = [
  'query', 'score', 'route', 'compose', 'act', 'verify', 'write', 'react',
] as const;

const TITLE = 'nunchi';
const LETTER_STAGGER_MS = 30;
const SUBTITLE_DELAY_MS = TITLE.length * LETTER_STAGGER_MS + 200;
const CTA_DELAY_MS = SUBTITLE_DELAY_MS + 400;

export default function Landing() {
  const [activeStep, setActiveStep] = useState(0);

  useEffect(() => {
    const id = setInterval(() => {
      setActiveStep((s) => (s + 1) % LOOP_PHASES.length);
    }, 2200);
    return () => clearInterval(id);
  }, []);

  return (
    <div className="landing-page">
      {/* 3D agent swarm background */}
      <AgentSwarm mode="chaos" />

      {/* ── HERO ── */}
      <div className="landing-hero">
        {/* NieR corners */}
        <div className="landing-corner landing-corner--tl" />
        <div className="landing-corner landing-corner--tr" />
        <div className="landing-corner landing-corner--bl" />
        <div className="landing-corner landing-corner--br" />

        {/* Full-width center content */}
        <div className="landing-hero-content">
          {/* Ornamental rule */}
          <div className="landing-rule" />

          {/* Gradient title with staggered letters */}
          <h1 className="landing-title-gradient">
            {TITLE.split('').map((char, i) => (
              <span
                key={i}
                className="landing-letter"
                style={{ animationDelay: `${i * LETTER_STAGGER_MS}ms` }}
              >
                {char}
              </span>
            ))}
          </h1>

          {/* Subtitle */}
          <p
            className="landing-subtitle"
            style={{ animationDelay: `${SUBTITLE_DELAY_MS}ms` }}
          >
            the agent coordination plane
          </p>

          {/* Lower rule */}
          <div className="landing-rule landing-rule--lower" />

          {/* Single START button */}
          <div
            className="landing-cta-row"
            style={{ animationDelay: `${CTA_DELAY_MS}ms` }}
          >
            <Link to="/demo" className="landing-cta">
              start
            </Link>
          </div>

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

        {/* Footer mark */}
        <div className="landing-footer-mark">
          18 crates &middot; one universal loop
        </div>
      </div>
    </div>
  );
}
