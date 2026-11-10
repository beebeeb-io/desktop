# pdf.js standard fonts

These files are copied unchanged from `pdfjs-dist@6.5.438` (`node_modules/pdfjs-dist/standard_fonts/`), the
version pinned in `package.json`. pdf.js reads them from `/pdfjs/standard_fonts/` (`standardFontDataUrl` in
`src/v2/sections/open/pdf/render.ts`), so a PDF's standard fonts are drawn from this app, never fetched from a
network.

Licence: Apache-2.0 (Mozilla pdf.js). The Liberation fonts carry their own licence in `LICENSE_LIBERATION`.

To update: `bun add -E pdfjs-dist@<version>`, then copy `node_modules/pdfjs-dist/standard_fonts/*` here and
change the version above.
