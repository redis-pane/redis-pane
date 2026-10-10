export const appName = 'redis-pane';
export const docsRoute = '/docs';

export const gitConfig = {
  user: 'redis-pane',
  repo: 'redis-pane',
  branch: 'main',
};

export const siteDescription = 'A terminal UI for browsing and editing Redis, with live updates.';

// Set from next.config.mjs, the single place the prefix is written.
export const basePath = process.env.NEXT_PUBLIC_BASE_PATH ?? '';
