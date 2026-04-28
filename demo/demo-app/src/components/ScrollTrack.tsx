import { useEffect, useRef } from 'react';

export default function ScrollTrack() {
  const spanRef = useRef<HTMLSpanElement>(null);

  useEffect(() => {
    function onScroll() {
      const h = document.documentElement.scrollHeight - innerHeight;
      if (spanRef.current && h > 0) {
        spanRef.current.style.width = ((scrollY / h) * 100).toFixed(2) + '%';
      }
    }
    window.addEventListener('scroll', onScroll, { passive: true });
    return () => window.removeEventListener('scroll', onScroll);
  }, []);

  return (
    <div
      style={{
        position: 'fixed',
        top: 0,
        left: 0,
        width: '100%',
        height: 2,
        zIndex: 9999,
        background: 'transparent',
      }}
    >
      <span
        ref={spanRef}
        style={{
          display: 'block',
          height: '100%',
          width: 0,
          background: 'linear-gradient(90deg, var(--rose-dim), var(--rose-glow))',
          transition: 'width .1s linear',
        }}
      />
    </div>
  );
}
