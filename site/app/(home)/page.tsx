import Link from 'next/link';
import { basePath, gitConfig, siteDescription } from '@/lib/shared';

// Minimal landing page. The full product page is a later task.
export default function HomePage() {
  return (
    <div className="flex flex-1 flex-col items-center justify-center gap-6 px-4 py-16 text-center">
      <h1 className="text-4xl font-bold tracking-tight sm:text-5xl">redis-pane</h1>
      <p className="max-w-xl text-lg text-fd-muted-foreground">{siteDescription}</p>
      <div className="flex flex-wrap justify-center gap-3">
        <Link
          href="/docs"
          className="rounded-full bg-fd-primary px-5 py-2 text-sm font-medium text-fd-primary-foreground hover:opacity-90"
        >
          Read the docs
        </Link>
        <a
          href={`https://github.com/${gitConfig.user}/${gitConfig.repo}`}
          className="rounded-full border px-5 py-2 text-sm font-medium hover:bg-fd-accent"
        >
          GitHub
        </a>
      </div>
      <img
        src={`${basePath}/images/demo.gif`}
        alt="Browsing a keyspace in redis-pane: filtering to a key, then watching it update on screen the moment another client changes it"
        className="w-full max-w-3xl rounded-lg border shadow-lg"
      />
    </div>
  );
}
