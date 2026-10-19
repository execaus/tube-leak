import { readFileSync } from 'node:fs'

const [path, tag] = process.argv.slice(2)
if (!path || !tag) throw new Error('Usage: check-manifest.mjs latest.json vX.Y.Z')
const manifest = JSON.parse(readFileSync(path, 'utf8'))
if (manifest.version !== tag.slice(1)) throw new Error(`Unexpected manifest version ${manifest.version}`)
for (const target of ['windows-x86_64', 'darwin-aarch64', 'darwin-x86_64', 'linux-x86_64']) {
  const item = manifest.platforms?.[target]
  if (!item?.signature || !item.url?.startsWith(`https://github.com/execaus/tube-leak/releases/download/${tag}/`)) {
    throw new Error(`Missing or invalid signed update for ${target}`)
  }
}
