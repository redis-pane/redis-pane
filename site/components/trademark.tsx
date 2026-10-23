import { trademarkNotice } from '@/lib/shared';

/** The Redis trademark disclaimer and attribution, on every page that names Redis. */
export function TrademarkNotice({ className = '' }: { className?: string }) {
  return <p className={`text-xs text-fd-muted-foreground ${className}`}>{trademarkNotice}</p>;
}
