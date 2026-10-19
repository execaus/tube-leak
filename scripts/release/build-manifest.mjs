import { readFileSync, writeFileSync } from 'node:fs'
import { join } from 'node:path'

const [assetsDir, releasesPath, tag] = process.argv.slice(2)
if (!assetsDir || !releasesPath || !/^v\d+\.\d+\.\d+$/.test(tag)) {
  throw new Error('Usage: build-manifest.mjs assets-dir releases.json vX.Y.Z')
}

const release = JSON.parse(readFileSync(releasesPath, 'utf8')).find((item) => item.tag_name === tag)
if (!release) throw new Error(`Release ${tag} was not found`)
const version = tag.slice(1)
const names = release.assets.map((asset) => asset.name)

function entry(pattern) {
  const matches = names.filter((name) => pattern.test(name))
  if (matches.length !== 1) throw new Error(`Expected one signed asset for ${pattern}, found ${matches.length}`)
  const signatureName = matches[0]
  const assetName = signatureName.slice(0, -4)
  if (!names.includes(assetName)) throw new Error(`Missing update asset ${assetName}`)
  const signature = readFileSync(join(assetsDir, signatureName), 'utf8').trim()
  if (!signature) throw new Error(`Empty signature for ${assetName}`)
  return {
    signature,
    url: `https://github.com/execaus/tube-leak/releases/download/${tag}/${encodeURIComponent(assetName)}`,
  }
}

const platforms = {
  'windows-x86_64': entry(new RegExp(`^tube-leak_${version}_windows_x64-setup\\.exe\\.sig$`)),
  'darwin-aarch64': entry(new RegExp(`^tube-leak_${version}_darwin_aarch64\\.app\\.tar\\.gz\\.sig$`)),
  'darwin-x86_64': entry(new RegExp(`^tube-leak_${version}_darwin_x64\\.app\\.tar\\.gz\\.sig$`)),
  'linux-x86_64': entry(new RegExp(`^tube-leak_${version}_linux_amd64\\.AppImage\\.sig$`)),
}
platforms['windows-x86_64-nsis'] = platforms['windows-x86_64']
platforms['linux-x86_64-appimage'] = platforms['linux-x86_64']

writeFileSync(join(assetsDir, 'latest.json'), `${JSON.stringify({
  version,
  notes: release.body ?? '',
  pub_date: new Date().toISOString(),
  platforms,
}, null, 2)}\n`)
