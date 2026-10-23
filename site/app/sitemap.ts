import type { MetadataRoute } from 'next';
import { source } from '@/lib/source';
import { siteUrl } from '@/lib/shared';

export const dynamic = 'force-static';

export default function sitemap(): MetadataRoute.Sitemap {
  // With trailingSlash, every page is served at <base>/<path>/.
  return [
    { url: siteUrl },
    ...source.getPages().map((page) => ({ url: new URL(`${page.url.replace(/^\//, '')}/`, siteUrl).toString() })),
  ];
}
