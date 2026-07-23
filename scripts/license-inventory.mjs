import { readFileSync } from 'node:fs'
import { spawnSync } from 'node:child_process'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const root = join(dirname(fileURLToPath(import.meta.url)), '..')
const metadata = spawnSync(
  'cargo',
  [
    'metadata',
    '--locked',
    '--format-version',
    '1',
    '--manifest-path',
    join(root, 'rust-backend', 'Cargo.toml'),
  ],
  { encoding: 'utf8', maxBuffer: 16 * 1024 * 1024 },
)

if (metadata.status !== 0) {
  process.stderr.write(metadata.stderr || String(metadata.error || 'cargo metadata failed'))
  process.exit(metadata.status || 1)
}

const cargoPackages = JSON.parse(metadata.stdout).packages
const cargoMissing = cargoPackages
  .filter((item) => !item.license && !item.license_file)
  .map((item) => `${item.name}@${item.version}`)

const lock = JSON.parse(readFileSync(join(root, 'frontend', 'package-lock.json'), 'utf8'))
const npmPackages = Object.entries(lock.packages || {})
  .filter(([path]) => path.startsWith('node_modules/'))
  .map(([path, item]) => ({
    name: item.name || path.replace(/^node_modules\//, ''),
    version: item.version || 'unknown',
    license: item.license,
  }))

const documentedNpmExceptions = new Set(['claygl@1.3.0'])
const npmMissing = npmPackages
  .filter((item) => !item.license)
  .map((item) => `${item.name}@${item.version}`)
  .filter((item) => !documentedNpmExceptions.has(item))

if (cargoMissing.length || npmMissing.length) {
  if (cargoMissing.length) {
    console.error(`Rust packages with missing license metadata: ${cargoMissing.join(', ')}`)
  }
  if (npmMissing.length) {
    console.error(`npm packages with missing license metadata: ${npmMissing.join(', ')}`)
  }
  process.exit(1)
}

console.log(
  `License metadata present for ${cargoPackages.length} Rust packages and ` +
    `${npmPackages.length} npm packages; documented exceptions: ` +
    `${[...documentedNpmExceptions].join(', ')}`,
)
