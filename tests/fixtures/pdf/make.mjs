// Writes tests/fixtures/pdf/two-pages.pdf: a two-page PDF with real text, no download and no dependency.
// Page 1 carries "terrace" and "Terrace"; page 2 carries "terrace" again, so a case-insensitive find of
// "terrace" counts 3 matches on 2 pages. Run: node tests/fixtures/pdf/make.mjs
import { writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))

const content = [
  'BT /F1 24 Tf 72 700 Td (The terrace is open.) Tj 0 -36 Td (Terrace view from the room.) Tj ET',
  'BT /F1 24 Tf 72 700 Td (The terrace is closed today.) Tj ET',
]

const objects = [
  '<< /Type /Catalog /Pages 2 0 R >>',
  `<< /Type /Pages /Kids [${content.map((_, i) => `${3 + i} 0 R`).join(' ')}] /Count ${content.length} >>`,
  ...content.map((_, i) => `<< /Type /Page /Parent 2 0 R /MediaBox [0 0 612 792] /Resources << /Font << /F1 ${3 + content.length} 0 R >> >> /Contents ${4 + content.length + i} 0 R >>`),
  '<< /Type /Font /Subtype /Type1 /BaseFont /Helvetica >>',
  ...content.map((stream) => `<< /Length ${Buffer.byteLength(stream, 'latin1')} >>\nstream\n${stream}\nendstream`),
]

let pdf = '%PDF-1.4\n'
const offsets = []
for (let i = 0; i < objects.length; i += 1) {
  offsets.push(Buffer.byteLength(pdf, 'latin1'))
  pdf += `${i + 1} 0 obj\n${objects[i]}\nendobj\n`
}
const xref = Buffer.byteLength(pdf, 'latin1')
pdf += `xref\n0 ${objects.length + 1}\n0000000000 65535 f \n`
for (const offset of offsets) pdf += `${String(offset).padStart(10, '0')} 00000 n \n`
pdf += `trailer\n<< /Size ${objects.length + 1} /Root 1 0 R >>\nstartxref\n${xref}\n%%EOF\n`

writeFileSync(join(here, 'two-pages.pdf'), Buffer.from(pdf, 'latin1'))
console.log(`wrote two-pages.pdf (${Buffer.byteLength(pdf, 'latin1')} bytes, ${content.length} pages)`)
