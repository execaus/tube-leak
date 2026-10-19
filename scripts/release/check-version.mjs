import { readFileSync } from 'node:fs'

const tag = process.env.GITHUB_REF_NAME
const pkg = JSON.parse(readFileSync('package.json', 'utf8'))
const config = JSON.parse(readFileSync('src-tauri/tauri.conf.json', 'utf8'))
const cargo = readFileSync('src-tauri/Cargo.toml', 'utf8').match(/^version = "([^"]+)"/m)?.[1]
if (tag !== `v${pkg.version}` || config.version !== pkg.version || cargo !== pkg.version) {
  throw new Error(`Tag ${tag} does not match package, Tauri and Cargo versions ${pkg.version}, ${config.version}, ${cargo}`)
}
