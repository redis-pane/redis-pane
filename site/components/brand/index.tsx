import type { SVGProps } from 'react';
import { wordmarkLetters, wordmarkViewBox } from './wordmark-paths';

// The brand: a >:_ icon and the redis:pane wordmark. The name is still
// redis-pane everywhere it is typed; the colon is only how the wordmark is set.
// See docs/DESIGN.md §5.1.

/** The >:_ icon on its tile. `small` is the heavier cut drawn for 32px and under. */
export function BrandIcon({ small = false, ...props }: SVGProps<SVGSVGElement> & { small?: boolean }) {
  return (
    <svg viewBox="0 0 64 64" aria-hidden="true" {...props}>
      {/* The hairline keeps the tile's edge on the dark theme, whose background is near its fill. */}
      <rect x="2" y="2" width="60" height="60" rx="14" fill="#16181C" stroke="var(--color-fd-border)" strokeWidth="1" />
      <g transform="translate(9.6 9.6) scale(0.7)" fill="#FAFAF8" stroke="#FAFAF8" strokeLinecap="round">
        {small ? (
          <>
            <path d="M9 20L21 32L9 44" fill="none" strokeWidth="7" strokeLinejoin="round" />
            <circle cx="32" cy="24" r="4.5" stroke="none" />
            <circle cx="32" cy="40" r="4.5" stroke="none" />
            <path d="M42 44H56" stroke="var(--color-brand)" strokeWidth="7" />
          </>
        ) : (
          <>
            <path d="M10 22L20 32L10 42" fill="none" strokeWidth="4.5" strokeLinejoin="round" />
            <circle cx="31" cy="25" r="3.2" stroke="none" />
            <circle cx="31" cy="39" r="3.2" stroke="none" />
            <path d="M40 42H54" stroke="var(--color-brand)" strokeWidth="4.5" />
          </>
        )}
      </g>
    </svg>
  );
}

/** redis:pane, outlined. Letters take the text colour; the colon is the brand accent. */
export function Wordmark(props: SVGProps<SVGSVGElement>) {
  return (
    <svg viewBox={wordmarkViewBox} aria-hidden="true" {...props}>
      {wordmarkLetters.map(({ d, accent }, i) => (
        <path key={i} d={d} fill={accent ? 'var(--color-brand)' : 'currentColor'} />
      ))}
    </svg>
  );
}
