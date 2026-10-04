// Copies the self-hosted woff2 files (latin + latin-ext) from the pinned Fontsource
// packages into public/fonts. Run with `npm run fonts` after bumping either package.
import { copyFileSync, mkdirSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const out = join(root, 'public', 'fonts')
mkdirSync(out, { recursive: true })

const sets = [
  { pkg: 'sora', weights: [400, 500, 600, 700] },
  { pkg: 'jetbrains-mono', weights: [400, 500] },
]
for (const { pkg, weights } of sets) {
  const dir = join(root, 'node_modules', '@fontsource', pkg)
  for (const subset of ['latin', 'latin-ext']) {
    for (const w of weights) {
      const file = `${pkg}-${subset}-${w}-normal.woff2`
      copyFileSync(join(dir, 'files', file), join(out, file))
    }
  }
  copyFileSync(join(dir, 'LICENSE'), join(out, `${pkg}-LICENSE.txt`))
}
