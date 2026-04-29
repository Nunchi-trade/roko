import './AsciiFrame.css';

interface AsciiFrameProps {
  variant?: 'single' | 'double' | 'rounded' | 'heavy';
  title?: string;
  status?: string;
  color?: string;
  children: React.ReactNode;
  className?: string;
}

interface BoxChars {
  tl: string; tr: string;
  bl: string; br: string;
  h: string;  v: string;
}

const BOX: Record<string, BoxChars> = {
  single:  { tl: '\u250C', tr: '\u2510', bl: '\u2514', br: '\u2518', h: '\u2500', v: '\u2502' },
  double:  { tl: '\u2554', tr: '\u2557', bl: '\u255A', br: '\u255D', h: '\u2550', v: '\u2551' },
  rounded: { tl: '\u256D', tr: '\u256E', bl: '\u2570', br: '\u256F', h: '\u2500', v: '\u2502' },
  heavy:   { tl: '\u250F', tr: '\u2513', bl: '\u2517', br: '\u251B', h: '\u2501', v: '\u2503' },
};

const HFILL_COUNT = 200;

export function AsciiFrame({
  variant = 'single',
  title,
  status,
  color,
  children,
  className,
}: AsciiFrameProps) {
  const ch = BOX[variant] ?? BOX.single;
  const hfill = ch.h.repeat(HFILL_COUNT);

  const style = color ? { color } as React.CSSProperties : undefined;
  const cls = ['ascii-frame', className ?? ''].filter(Boolean).join(' ');

  return (
    <div className={cls} style={style}>
      {/* top border */}
      <div className="ascii-frame__top" aria-hidden="true">
        <span className="ascii-frame__corner">{ch.tl}</span>
        {title && (
          <>
            <span className="ascii-frame__hfill">{ch.h}{ch.h}</span>
            <span className="ascii-frame__title">{title}</span>
          </>
        )}
        <span className="ascii-frame__hfill">{hfill}</span>
        {status && (
          <>
            <span className="ascii-frame__status">{status}</span>
            <span className="ascii-frame__hfill">{ch.h}{ch.h}</span>
          </>
        )}
        <span className="ascii-frame__corner">{ch.tr}</span>
      </div>

      {/* body with vertical bars */}
      <div className="ascii-frame__body">
        <span className="ascii-frame__vbar" aria-hidden="true">{ch.v}</span>
        <div className="ascii-frame__content">{children}</div>
        <span className="ascii-frame__vbar" aria-hidden="true">{ch.v}</span>
      </div>

      {/* bottom border */}
      <div className="ascii-frame__bottom" aria-hidden="true">
        <span className="ascii-frame__corner">{ch.bl}</span>
        <span className="ascii-frame__hfill">{hfill}</span>
        <span className="ascii-frame__corner">{ch.br}</span>
      </div>
    </div>
  );
}
