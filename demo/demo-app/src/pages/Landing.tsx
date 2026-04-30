import { useState, useEffect, useRef } from 'react';
import { Link } from 'react-router';
import HeroScene from '../components/HeroScene';
import '../components/HeroScene/HeroScene.css';
import './Landing.css';

const LOOP_PHASES = [
  'query', 'score', 'route', 'compose', 'act', 'verify', 'write', 'react',
] as const;

const TITLE = 'nunchi';
const LETTER_STAGGER_MS = 30;
const SUBTITLE_DELAY_MS = TITLE.length * LETTER_STAGGER_MS + 200;
const CTA_DELAY_MS = SUBTITLE_DELAY_MS + 400;

const FEATURES = [
  {
    label: '01',
    title: 'Universal Loop',
    desc: 'Query, score, route, compose, act, verify, write, react. One pattern for every agent.',
  },
  {
    label: '02',
    title: '18 Crates',
    desc: 'Modular Rust toolkit: core primitives, agent dispatch, gate pipeline, learning, and more.',
  },
  {
    label: '03',
    title: 'Self-Hosting',
    desc: 'Roko reads PRDs, generates plans, executes tasks via agents, validates with gates, and iterates.',
  },
];

/** IntersectionObserver hook for scroll reveal */
function useScrollReveal() {
  const ref = useRef<HTMLDivElement>(null);
  const [visible, setVisible] = useState(false);

  useEffect(() => {
    const el = ref.current;
    if (!el) return;
    const observer = new IntersectionObserver(
      ([entry]) => {
        if (entry.isIntersecting) {
          setVisible(true);
          observer.unobserve(el);
        }
      },
      { threshold: 0.15 },
    );
    observer.observe(el);
    return () => observer.disconnect();
  }, []);

  return { ref, visible };
}

/** Parallax offset from scroll position */
function useParallax() {
  const [scrollY, setScrollY] = useState(0);
  useEffect(() => {
    function onScroll() {
      setScrollY(window.scrollY);
    }
    window.addEventListener('scroll', onScroll, { passive: true });
    return () => window.removeEventListener('scroll', onScroll);
  }, []);
  return scrollY;
}

export default function Landing() {
  const [activeStep, setActiveStep] = useState(0);
  const scrollY = useParallax();
  const featuresReveal = useScrollReveal();
  const quoteReveal = useScrollReveal();

  useEffect(() => {
    const id = setInterval(() => {
      setActiveStep((s) => (s + 1) % LOOP_PHASES.length);
    }, 2200);
    return () => clearInterval(id);
  }, []);

  // Parallax rates: title moves slower, 3D scene moves faster
  const titleOffset = scrollY * 0.3;
  const sceneOffset = scrollY * 0.6;

  return (
    <div className="landing-page">
      {/* ── HERO ── */}
      <div className="landing-hero">
        {/* 3D scene with parallax */}
        <div style={{ transform: `translateY(${sceneOffset * 0.2}px)`, width: '100%', height: '100%', position: 'absolute', inset: 0 }}>
          <HeroScene activeStep={activeStep} />
        </div>

        {/* Floating ambient shapes */}
        <div className="landing-float landing-float--circle-1" />
        <div className="landing-float landing-float--circle-2" />
        <div className="landing-float landing-float--line-1" />
        <div className="landing-float landing-float--diamond" />
        <div className="landing-float landing-float--line-2" />

        {/* NieR corners */}
        <div className="landing-corner landing-corner--tl" />
        <div className="landing-corner landing-corner--tr" />
        <div className="landing-corner landing-corner--bl" />
        <div className="landing-corner landing-corner--br" />

        {/* Center content with parallax */}
        <div
          className="landing-hero-content"
          style={{ transform: `translateY(${titleOffset * 0.15}px)` }}
        >
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

          {/* Subtitle fades up after title */}
          <p
            className="landing-subtitle"
            style={{ animationDelay: `${SUBTITLE_DELAY_MS}ms` }}
          >
            the agent coordination plane
          </p>

          {/* Lower rule */}
          <div className="landing-rule landing-rule--lower" />

          {/* CTA buttons with bounce entrance */}
          <div
            className="landing-cta-row"
            style={{ animationDelay: `${CTA_DELAY_MS}ms` }}
          >
            <Link to="/demo" className="landing-cta">
              start
            </Link>
            <Link to="/dashboard" className="landing-cta landing-cta--secondary">
              dashboard
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

        {/* Scroll hint */}
        <div className="landing-scroll-hint" />

        {/* Footer mark */}
        <div className="landing-footer-mark">
          18 crates &middot; one universal loop
        </div>
      </div>

      {/* ── SCROLL SECTIONS ── */}
      <div className="landing-sections">
        {/* Features grid */}
        <div className="landing-section">
          <div
            ref={featuresReveal.ref}
            className={`landing-reveal ${featuresReveal.visible ? 'visible' : ''}`}
          >
            <div className="stag" style={{ marginBottom: 'var(--sp-5)' }}>
              <span className="num">01</span>
              <span className="label">Architecture</span>
            </div>
            <div className="landing-features">
              {FEATURES.map((f) => (
                <div key={f.label} className="landing-feature">
                  <div className="landing-feature__label">{f.label}</div>
                  <div className="landing-feature__title">{f.title}</div>
                  <div className="landing-feature__desc">{f.desc}</div>
                </div>
              ))}
            </div>
          </div>
        </div>

        {/* Quote */}
        <div className="landing-section">
          <div
            ref={quoteReveal.ref}
            className={`landing-reveal ${quoteReveal.visible ? 'visible' : ''}`}
          >
            <div className="landing-quote">
              <div className="landing-quote__label">Axiom</div>
              <div className="landing-quote__text">
                One noun, six verbs. <em>Signal</em> flows through Substrate, Scorer,
                Gate, Router, Composer, and Policy — the universal loop that builds itself.
              </div>
            </div>
          </div>
        </div>
      </div>
    </div>
  );
}
