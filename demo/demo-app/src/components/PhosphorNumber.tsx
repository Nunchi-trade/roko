import { useEffect, useRef, useState } from 'react';

interface PhosphorNumberProps {
  value: number;
  format?: (n: number) => string;
  className?: string;
}

export default function PhosphorNumber({ value, format, className }: PhosphorNumberProps) {
  const [flash, setFlash] = useState(false);
  const prevRef = useRef(value);

  useEffect(() => {
    if (value !== prevRef.current) {
      prevRef.current = value;
      setFlash(true);
      const t = setTimeout(() => setFlash(false), 600);
      return () => clearTimeout(t);
    }
  }, [value]);

  const display = format ? format(value) : String(value);

  return (
    <span
      className={className}
      style={{
        transition: 'color .3s ease, text-shadow .6s ease',
        color: flash ? 'var(--rose-bright, #dc9cb8)' : undefined,
        textShadow: flash ? '0 0 12px color-mix(in srgb, var(--rose) 50%, transparent)' : 'none',
      }}
    >
      {display}
    </span>
  );
}
