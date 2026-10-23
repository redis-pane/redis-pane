'use client';

import { useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { installCommands, links } from '@/lib/release';

// Homebrew first: on macOS it is also the install that skips Gatekeeper's first-run prompt.
const tabs = [
  { id: 'homebrew', label: 'Homebrew', prompt: '$ ' },
  { id: 'shell', label: 'Shell script', prompt: '$ ' },
  { id: 'powershell', label: 'Windows', prompt: 'PS> ' },
] as const;

export function InstallCommand() {
  const [tab, setTab] = useState<(typeof tabs)[number]['id']>('homebrew');
  const [copied, setCopied] = useState(false);
  const command = installCommands[tab];

  async function copy() {
    try {
      await navigator.clipboard.writeText(command);
      setCopied(true);
      setTimeout(() => setCopied(false), 1800);
    } catch {
      // Clipboard unavailable (insecure context); the text stays selectable.
    }
  }

  return (
    <div className="w-full max-w-3xl overflow-hidden rounded-xl border bg-fd-card text-left shadow-sm">
      <div className="flex items-center justify-between border-b px-2">
        <div role="tablist" className="flex">
          {tabs.map((t) => (
            <button
              key={t.id}
              role="tab"
              aria-selected={tab === t.id}
              onClick={() => setTab(t.id)}
              className={`border-b-2 px-3 py-2.5 text-xs font-medium transition-colors ${
                tab === t.id
                  ? 'border-fd-foreground text-fd-foreground'
                  : 'border-transparent text-fd-muted-foreground hover:text-fd-foreground'
              }`}
            >
              {t.label}
            </button>
          ))}
        </div>
        <button
          onClick={copy}
          aria-label="Copy install command"
          className="inline-flex items-center gap-1.5 rounded-md px-2 py-1.5 text-xs text-fd-muted-foreground transition-colors hover:bg-fd-accent hover:text-fd-foreground"
        >
          {copied ? <Check className="size-3.5" /> : <Copy className="size-3.5" />}
          {copied ? 'Copied' : 'Copy'}
        </button>
      </div>
      <pre className="overflow-x-auto px-4 py-4 font-mono text-[13px] leading-relaxed">
        <code>
          <span className="select-none text-fd-muted-foreground">
            {tabs.find((t) => t.id === tab)?.prompt}
          </span>
          {command}
        </code>
      </pre>
      <p className="border-t px-4 py-2 text-xs text-fd-muted-foreground">
        {tab === 'homebrew' && 'macOS and Linux.'}
        {tab === 'shell' && 'macOS and Linux, without Homebrew. Linux gets a static build that runs on any distro.'}
        {tab === 'powershell' && (
          <>
            Experimental: config files, session restore and <code>passwordCommand</code> don&apos;t
            work yet on Windows (
            <a href={links.windowsIssue} className="underline hover:text-fd-foreground">
              #138
            </a>
            ).
          </>
        )}
      </p>
    </div>
  );
}
