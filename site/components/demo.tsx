import { basePath } from '@/lib/shared';

/**
 * A recording from tapes/, rendered by tapes/render.sh into public/demos/.
 * `name` is the tape's name: public/demos/<name>.{webm,mp4,png}.
 * The paths carry basePath, which static files in public/ do not get for free.
 */
export function DemoVideo({ name, alt, className }: { name: string; alt: string; className?: string }) {
  const src = `${basePath}/demos/${name}`;
  return (
    <video autoPlay muted loop playsInline preload="metadata" poster={`${src}.png`} aria-label={alt} className={className} style={{ aspectRatio: "5 / 3" }}>
      <source src={`${src}.webm`} type="video/webm" />
      <source src={`${src}.mp4`} type="video/mp4" />
    </video>
  );
}

export function Demo({ name, alt, caption }: { name: string; alt: string; caption?: string }) {
  return (
    <figure className="my-6">
      <DemoVideo name={name} alt={alt} className="block w-full rounded-lg border" />
      {caption ? <figcaption className="mt-2 text-center text-sm text-fd-muted-foreground">{caption}</figcaption> : null}
    </figure>
  );
}
