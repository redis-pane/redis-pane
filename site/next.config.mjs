import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

const basePath = '/redis-pane';

/** @type {import('next').NextConfig} */
const config = {
  // Static export for GitHub Pages, served from https://redis-pane.github.io/redis-pane/.
  output: 'export',
  basePath,
  // The static search client needs the prefix too, which it does not add itself.
  env: { NEXT_PUBLIC_BASE_PATH: basePath },
  // /docs/x/ is /docs/x/index.html, which Pages serves without rewrites.
  trailingSlash: true,
  images: { unoptimized: true },
  reactStrictMode: true,
  turbopack: { root: import.meta.dirname },
};

export default withMDX(config);
