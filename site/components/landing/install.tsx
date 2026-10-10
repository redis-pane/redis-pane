'use client';

import { useState } from 'react';
import { Check, Copy } from 'lucide-react';
import { installCommands } from '@/lib/release';

const tabs = [
  { id: 'shell', label: 'macOS / Linux' },
  { id: 'powershell', label: 'PowerShell' },
] as const;

export function InstallCommand() {
  const [tab, setTab] = useState<(typeof tabs)[number]['id']>('shell');
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
          <span className="select-none text-fd-muted-foreground">$ </span>
          {command}
        </code>
      </pre>
    </div>
  );
}
