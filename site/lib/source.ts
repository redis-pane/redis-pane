import { loader } from 'fumadocs-core/source';
import { docsRoute } from './shared';
import { defineDocs } from 'fumadocs-mdx/macro';
import { applyMdxPreset } from 'fumadocs-mdx/config';
import { remarkJsonInvalid } from './remark-json-invalid';
import { metaSchema, pageSchema } from 'fumadocs-core/source/schema';

// Pages are plain .md (the generated reference pages must stay free of MDX escaping).
const docs = defineDocs({
  dir: 'content/docs',
  docs: {
    schema: pageSchema,
    mdxOptions: applyMdxPreset({ remarkPlugins: [remarkJsonInvalid] }),
  },
  meta: { schema: metaSchema },
});

export const source = loader({
  baseUrl: docsRoute,
  source: docs.toFumadocsSource(),
});
