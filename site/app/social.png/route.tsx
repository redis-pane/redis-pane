import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { ImageResponse } from 'next/og';

// Served at /social.png rather than as app/opengraph-image: the static export writes that one
// without a file extension, and GitHub Pages would send it as application/octet-stream.
export const dynamic = 'force-static';
const size = { width: 1280, height: 640 };

const asDataUri = async (file: string, mime: string) =>
  `data:${mime};base64,${(await readFile(path.join(process.cwd(), 'public', file))).toString('base64')}`;

export async function GET() {
  const lockup = await asDataUri('brand/lockup-dark.svg', 'image/svg+xml');
  const hero = await asDataUri('demos/hero.png', 'image/png');

  return new ImageResponse(
    (
      <div
        style={{
          width: '100%',
          height: '100%',
          display: 'flex',
          flexDirection: 'column',
          background: '#16181C',
          color: '#E9EBEE',
          padding: '56px 64px 0 64px',
        }}
      >
        {/* eslint-disable-next-line @next/next/no-img-element */}
        <img src={lockup} width={420} height={73} alt="" />
        <div style={{ display: 'flex', fontSize: 48, fontWeight: 700, lineHeight: 1.15, marginTop: 36, maxWidth: 1000 }}>
          A terminal UI for Redis and Valkey. Watch keys update live, edit safely, over SSH too.
        </div>
        <div
          style={{
            display: 'flex',
            marginTop: 'auto',
            marginLeft: 120,
            border: '2px solid #2E333B',
            borderBottom: 'none',
            borderTopLeftRadius: 14,
            borderTopRightRadius: 14,
            overflow: 'hidden',
            height: 250,
          }}
        >
          {/* eslint-disable-next-line @next/next/no-img-element */}
          <img src={hero} width={1000} height={600} alt="" style={{ objectFit: 'cover', objectPosition: 'top' }} />
        </div>
        <div style={{ position: 'absolute', left: 0, right: 0, bottom: 0, height: 8, background: '#1AA088', display: 'flex' }} />
      </div>
    ),
    size,
  );
}
