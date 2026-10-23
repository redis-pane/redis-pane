export const appName = 'redis-pane';
export const docsRoute = '/docs';

export const gitConfig = {
  user: 'redis-pane',
  repo: 'redis-pane',
  branch: 'main',
};

export const siteDescription =
  'A terminal UI for Redis and Valkey: browse a keyspace, watch keys update live, and edit safely, over SSH too.';

// The deployed origin and base path. Absolute URLs (sitemap, social card) are built from these.
export const siteOrigin = 'https://redis-pane.github.io';
export const siteUrl = `${siteOrigin}/redis-pane/`;

// Set from next.config.mjs, the single place the prefix is written.
export const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '';
