const fs = require('node:fs')
const path = require('node:path')
const { spawnSync } = require('node:child_process')

const root = path.resolve(__dirname, '..')
const browser = process.argv[2] === '--browser'
const output = path.resolve(
  process.argv[browser ? 3 : 2] ||
    path.join(root, browser ? 'pkg/browser' : 'pkg'),
)

function run(command, args, capture = false) {
  const result = spawnSync(command, args, {
    cwd: root,
    encoding: 'utf8',
    stdio: capture ? ['ignore', 'pipe', 'inherit'] : 'inherit',
    maxBuffer: 16 * 1024 * 1024,
  })
  if (result.error) throw result.error
  if (result.status !== 0)
    throw new Error(command + ' exited with ' + result.status)
  return result.stdout
}

run('cargo', [
  'build',
  '--locked',
  '--release',
  '--lib',
  '--target',
  'wasm32-unknown-unknown',
])
const metadata = JSON.parse(
  run(
    'cargo',
    [
      'metadata',
      '--locked',
      '--offline',
      '--format-version',
      '1',
      '--filter-platform',
      'wasm32-unknown-unknown',
    ],
    true,
  ),
)
run('wasm-bindgen', [
  path.join(
    metadata.target_directory,
    'wasm32-unknown-unknown/release/ncm_recognition.wasm',
  ),
  '--out-dir',
  output,
  '--target',
  browser ? 'no-modules' : 'web',
  ...(browser ? ['--no-modules-global', 'NcmRecognition'] : []),
])

console.log('Built WASM, browser bindings in ' + output)
