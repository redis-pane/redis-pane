export const appName = 'redis-pane';
export const docsRoute = '/docs';

export const gitConfig = {
  user: 'redis-pane',
  repo: 'redis-pane',
  branch: 'main',
};

export const siteDescription =
  'A terminal UI for the Redis® database and Valkey: browse a keyspace, watch keys update live, and edit safely, over SSH too.';

// The disclaimer and attribution the Redis trademark policy asks for on every page that names Redis
// (https://redis.io/legal/trademark-policy/, its open-source section). Rendered by
// components/trademark.tsx on every docs page and the landing page, and repeated in the README.
export const trademarkNotice =
  'redis-pane is an independent open-source project. It is not affiliated with, endorsed, supported or certified by Redis Ltd. Redis is a registered trademark of Redis Ltd. Any rights therein are reserved to Redis Ltd.';

// The deployed origin and base path. Absolute URLs (sitemap, social card) are built from these.
export const siteOrigin = 'https://redis-pane.github.io';
export const siteUrl = `${siteOrigin}/redis-pane/`;

// Set from next.config.mjs, the single place the prefix is written.
export const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '';
