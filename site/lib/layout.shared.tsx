import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';
import { BrandIcon, Wordmark } from '@/components/brand';
import { appName, gitConfig } from './shared';

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: (
        <>
          <BrandIcon small className="size-6" />
          <Wordmark className="h-3.5 w-auto" />
          <span className="sr-only">{appName}</span>
        </>
      ),
    },
    githubUrl: `https://github.com/${gitConfig.user}/${gitConfig.repo}`,
  };
}
