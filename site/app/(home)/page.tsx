import Link from 'next/link';
import {
  ArrowRight,
  Boxes,
  FolderTree,
  Gauge,
  MonitorSmartphone,
  PencilLine,
  Radio,
} from 'lucide-react';
import { InstallCommand } from '@/components/landing/install';
import { links, releaseVersion } from '@/lib/release';
import { basePath, siteDescription } from '@/lib/shared';

const docs = (path: string) => `/docs/${path}`;

const features = [
  {
    icon: FolderTree,
    title: 'Browse',
    text: 'Keys stream in with SCAN, never KEYS, and the list is usable while they arrive. Tree or flat, filter, five sort orders.',
    href: docs('browsing/key-list'),
  },
  {
    icon: Radio,
    title: 'See it live',
    text: 'The open key updates the moment another client changes it, pushed by the server. If that ever stops, the header says so.',
    href: docs('viewing/live-updates'),
  },
  {
    icon: PencilLine,
    title: 'Edit safely',
    text: 'Every write shows the exact command first. prod and unknown targets start in Read-only Mode.',
    href: docs('editing/values'),
  },
  {
    icon: Gauge,
    title: 'Watch the server',
    text: 'A Dashboard, a live Monitor tail, Pub/Sub and the Slowlog, each a keystroke away.',
    href: docs('server/dashboard'),
  },
  {
    icon: Boxes,
    title: 'Cluster & Sentinel',
    text: 'Point it at a cluster and the key list covers every primary. A Sentinel URL resolves to the current master.',
    href: docs('connecting/cluster'),
  },
  {
    icon: MonitorSmartphone,
    title: 'Fits any terminal',
    text: 'Dark, light and high-contrast themes, an ASCII fallback, and a layout that works down to 80×24.',
    href: docs('appearance/terminal-sizes'),
  },
];

const footnotes: [string, ...string[]][] = [
  ['RedisInsight describes itself as a "desktop GUI client" built on Electron, also shipped as a Docker image. It has no terminal interface.'],
  ['The `redis-pane-aarch64-apple-darwin.tar.xz` asset of release `0.1.0-beta.4` is 2.7 MB. The four platform downloads range from 2.7 MB to 5.5 MB (the Windows zip).'],
  ['An Electron desktop application, or a Docker image, per its README.'],
  ["Its documentation describes a configurable automatic refresh rate for Streams and for the Slow Log. It documents no push update for an open key's value."],
  ['`crates/core/tests/perf.rs` drives 1,000,000 keys in real `SCAN` order. No single update, which includes any slice of a rebuild, takes longer than the 16 ms frame budget.'],
  ['`redis-cli --scan` streams the names of every key, with `--pattern` to filter. It doesn\'t browse, show types or open values.'],
  ['We have not measured other tools and make no claim about them.'],
  ['`redis-cli` runs what you type. Nothing previews it and nothing locks it.'],
  ['We found no mention of a command preview, a read-only mode or an Environment lock in its README or documentation. We have not tested it, so this is "not documented", not "absent".'],
  ['`redis-cli -c` follows `-MOVED` and `-ASK` redirections, and `--cluster` runs the cluster manager commands.'],
  ['Its documentation says you can add "any Redis database running anywhere (including Redis Open Source cluster or sentinel)".'],
  ['`redis-cli --help` has no Sentinel option. A Sentinel node can be queried as an ordinary server.'],
];

// Cell: [symbol, footnote number (1-based) or undefined, extra text]
type Cell = { v: string; n?: number; note?: string };
const rows: { label: string; cells: [Cell, Cell, Cell] }[] = [
  { label: 'Runs in a terminal and over SSH', cells: [{ v: '✓' }, { v: '✓' }, { v: '✗', n: 1 }] },
  { label: 'Single binary', cells: [{ v: '✓', note: '2.7 MB download', n: 2 }, { v: '✓' }, { v: '✗', n: 3 }] },
  { label: 'The open key updates live, by push, with no refresh', cells: [{ v: '✓' }, { v: '✗' }, { v: 'partial', n: 4 }] },
  { label: 'A million keys browsable', cells: [{ v: '✓', note: 'measured', n: 5 }, { v: 'partial', n: 6 }, { v: 'not measured', n: 7 }] },
  { label: 'The exact command is shown before every write', cells: [{ v: '✓' }, { v: '✗', n: 8 }, { v: 'not documented', n: 9 }] },
  { label: 'Writes are locked by Environment', cells: [{ v: '✓' }, { v: '✗', n: 8 }, { v: 'not documented', n: 9 }] },
  { label: 'Cluster', cells: [{ v: '✓' }, { v: '✓', n: 10 }, { v: '✓', n: 11 }] },
  { label: 'Sentinel', cells: [{ v: '✓' }, { v: '✗', n: 12 }, { v: '✓', n: 11 }] },
];

// Renders `code` spans inside a footnote.
function Inline({ text }: { text: string }) {
  return text.split(/(`[^`]+`)/).map((part, i) =>
    part.startsWith('`') ? (
      <code key={i} className="rounded bg-fd-muted px-1 py-0.5 font-mono text-[0.85em]">
        {part.slice(1, -1)}
      </code>
    ) : (
      part
    ),
  );
}

function CellView({ cell, strong }: { cell: Cell; strong?: boolean }) {
  const tone =
    cell.v === '✓' ? (strong ? 'text-fd-foreground font-semibold' : 'text-fd-foreground') : 'text-fd-muted-foreground';
  return (
    <span className={tone}>
      {cell.v}
      {cell.note && <span className="text-fd-muted-foreground"> ({cell.note}{cell.n ? <Ref n={cell.n} /> : null})</span>}
      {!cell.note && cell.n ? <Ref n={cell.n} /> : null}
    </span>
  );
}

function Ref({ n }: { n: number }) {
  return (
    <sup className="ml-0.5">
      <a href={`#fn-${n}`} className="text-fd-muted-foreground hover:text-fd-foreground">
        [{n}]
      </a>
    </sup>
  );
}

const limits = [
  'No Redis console. There is nowhere to type a raw command, so it doesn\'t replace redis-cli in scripts.',
  'One Connection means one database, fixed at launch. No moving a key to another database.',
  "Values aren't opened in your $EDITOR, and values over 200 KB can't be edited.",
  'Streams are a read-only list of the newest 500 entries. Keys beyond 2,000,000 aren\'t loaded.',
  "Not a server manager, an alerting tool or a metrics store, and no multi-server workspace.",
];

export default function HomePage() {
  return (
    <main className="flex flex-1 flex-col">
      {/* Hero */}
      <section className="relative overflow-hidden border-b">
        <div
          aria-hidden
          className="pointer-events-none absolute inset-0 -z-10 bg-[radial-gradient(ellipse_at_top,var(--color-fd-accent),transparent_65%)] opacity-60"
        />
        <div className="mx-auto flex max-w-5xl flex-col items-center gap-7 px-4 pb-16 pt-20 text-center sm:pt-28">
          <span className="rounded-full border bg-fd-card px-3 py-1 text-xs text-fd-muted-foreground">
            v{releaseVersion} · beta
          </span>
          <h1 className="text-balance text-4xl font-semibold tracking-tight sm:text-6xl">redis-pane</h1>
          <p className="max-w-xl text-balance text-lg text-fd-muted-foreground sm:text-xl">
            A terminal UI for Redis: see your keyspace, open a key, watch it change, edit it safely.
          </p>
          <InstallCommand />
          <div className="flex flex-wrap justify-center gap-3">
            <Link
              href="/docs"
              className="inline-flex items-center gap-2 rounded-full bg-fd-primary px-5 py-2.5 text-sm font-medium text-fd-primary-foreground transition-opacity hover:opacity-90"
            >
              Get started <ArrowRight className="size-4" />
            </Link>
            <a
              href={links.repo}
              className="inline-flex items-center rounded-full border bg-fd-background px-5 py-2.5 text-sm font-medium transition-colors hover:bg-fd-accent"
            >
              GitHub
            </a>
          </div>
        </div>
      </section>

      {/* Demo */}
      <section className="mx-auto w-full max-w-5xl px-4 py-16">
        <figure>
          <div className="overflow-hidden rounded-xl border bg-fd-card shadow-xl">
            <div className="flex items-center gap-2 border-b bg-fd-muted/50 px-4 py-3">
              <span className="size-3 rounded-full bg-fd-border" />
              <span className="size-3 rounded-full bg-fd-border" />
              <span className="size-3 rounded-full bg-fd-border" />
              <span className="ml-3 truncate font-mono text-xs text-fd-muted-foreground">redis-pane</span>
            </div>
            {/* eslint-disable-next-line @next/next/no-img-element */}
            <img
              src={`${basePath}/images/demo.gif`}
              alt="Browsing a keyspace in redis-pane: filtering to a key, then watching it update on screen the moment another client changes it, with no keypress or refresh"
              className="block w-full"
            />
          </div>
          <figcaption className="mt-4 text-center text-sm text-fd-muted-foreground">
            Filter to a key, open it, and watch it update live the moment it changes on the server. There is no
            refresh button.
          </figcaption>
        </figure>
      </section>

      {/* Features */}
      <section className="mx-auto w-full max-w-5xl px-4 pb-16">
        <h2 className="mb-8 text-2xl font-semibold tracking-tight sm:text-3xl">Everything in the terminal you are already in</h2>
        <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-3">
          {features.map(({ icon: Icon, title, text, href }) => (
            <Link
              key={title}
              href={href}
              className="group flex flex-col gap-3 rounded-xl border bg-fd-card p-5 transition-colors hover:bg-fd-accent/50"
            >
              <span className="flex size-9 items-center justify-center rounded-lg border bg-fd-background">
                <Icon className="size-4.5" />
              </span>
              <h3 className="font-semibold">{title}</h3>
              <p className="flex-1 text-sm leading-relaxed text-fd-muted-foreground">{text}</p>
              <span className="inline-flex items-center gap-1 text-sm text-fd-muted-foreground transition-colors group-hover:text-fd-foreground">
                Read the docs <ArrowRight className="size-3.5 transition-transform group-hover:translate-x-0.5" />
              </span>
            </Link>
          ))}
        </div>
      </section>

      {/* Comparison */}
      <section className="mx-auto w-full max-w-5xl px-4 pb-16">
        <h2 className="mb-2 text-2xl font-semibold tracking-tight sm:text-3xl">Why</h2>
        <p className="mb-8 max-w-2xl text-fd-muted-foreground">
          <code className="rounded bg-fd-muted px-1 py-0.5 font-mono text-[0.85em]">redis-cli</code> runs commands but can&apos;t
          show you what is in your keyspace. RedisInsight can, but it is a desktop GUI, so it is no use in an SSH session on a
          bastion host. redis-pane is one small binary that runs in the terminal you are already in, driven from the keyboard.
        </p>
        <div className="overflow-x-auto rounded-xl border">
          <table className="w-full min-w-[640px] border-collapse text-sm">
            <thead>
              <tr className="border-b bg-fd-muted/40 text-left">
                <th className="px-4 py-3 font-medium" />
                <th className="px-4 py-3 text-center font-semibold">redis-pane</th>
                <th className="px-4 py-3 text-center font-medium text-fd-muted-foreground">
                  <code className="font-mono">redis-cli</code>
                </th>
                <th className="px-4 py-3 text-center font-medium text-fd-muted-foreground">RedisInsight</th>
              </tr>
            </thead>
            <tbody>
              {rows.map((r) => (
                <tr key={r.label} className="border-b last:border-0">
                  <th scope="row" className="px-4 py-3 text-left font-normal">
                    {r.label}
                  </th>
                  {r.cells.map((c, i) => (
                    <td key={i} className={`px-4 py-3 text-center ${i === 0 ? 'bg-fd-accent/40' : ''}`}>
                      <CellView cell={c} strong={i === 0} />
                    </td>
                  ))}
                </tr>
              ))}
            </tbody>
          </table>
        </div>
        <ol className="mt-6 space-y-1.5 text-xs leading-relaxed text-fd-muted-foreground">
          {footnotes.map(([t], i) => (
            <li key={i} id={`fn-${i + 1}`} className="flex gap-2 scroll-mt-20">
              <span className="w-6 shrink-0 text-right tabular-nums">[{i + 1}]</span>
              <span>
                <Inline text={t} />
              </span>
            </li>
          ))}
        </ol>
      </section>

      {/* Limits */}
      <section className="mx-auto w-full max-w-5xl px-4 pb-20">
        <h2 className="mb-6 text-2xl font-semibold tracking-tight sm:text-3xl">What it doesn&apos;t do</h2>
        <ul className="grid gap-x-10 gap-y-2.5 text-sm text-fd-muted-foreground sm:grid-cols-2">
          {limits.map((l) => (
            <li key={l} className="flex gap-2">
              <span aria-hidden className="mt-2 size-1 shrink-0 rounded-full bg-fd-muted-foreground" />
              {l}
            </li>
          ))}
        </ul>
        <Link
          href="/docs/limits"
          className="mt-6 inline-flex items-center gap-1 text-sm font-medium hover:underline"
        >
          Read the full list: Limits and non-goals <ArrowRight className="size-3.5" />
        </Link>
      </section>

      {/* Footer */}
      <footer className="border-t">
        <div className="mx-auto flex max-w-5xl flex-col gap-4 px-4 py-8 text-sm text-fd-muted-foreground sm:flex-row sm:items-center sm:justify-between">
          <p>{siteDescription}</p>
          <nav className="flex flex-wrap gap-x-6 gap-y-2">
            <Link href="/docs" className="hover:text-fd-foreground">Docs</Link>
            <a href={links.releases} className="hover:text-fd-foreground">Releases</a>
            <a href={links.discussions} className="hover:text-fd-foreground">Discussions</a>
            <a href={links.license} className="hover:text-fd-foreground">MIT license</a>
          </nav>
        </div>
      </footer>
    </main>
  );
}
