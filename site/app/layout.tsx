import type { Metadata } from 'next';
import { Provider } from '@/components/provider';
import { appName, siteDescription, siteUrl } from '@/lib/shared';
import './global.css';

// The social card is rendered by app/social.png/route.tsx and written to out/social.png.
const socialImage = {
  url: '/social.png',
  width: 1280,
  height: 640,
  alt: 'redis-pane: a terminal UI for the Redis® database and Valkey',
};

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl),
  title: { default: appName, template: `%s | ${appName}` },
  description: siteDescription,
  openGraph: {
    type: 'website',
    siteName: appName,
    title: appName,
    description: siteDescription,
    url: siteUrl,
    images: [socialImage],
  },
  twitter: {
    card: 'summary_large_image',
    title: appName,
    description: siteDescription,
    images: [socialImage],
  },
};

export default function Layout({ children }: LayoutProps<'/'>) {
  return (
    <html lang="en" suppressHydrationWarning>
      <body className="flex flex-col min-h-screen">
        <Provider>{children}</Provider>
      </body>
    </html>
  );
}
