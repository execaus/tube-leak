import { afterEach, describe, expect, it } from 'vitest'

import { KNOWN_TARGETS, resolveHostTarget } from './targets.mjs'

const originalPlatform = process.platform
const originalArch = process.arch

function stubHost(platform, arch) {
  Object.defineProperty(process, 'platform', { value: platform, configurable: true })
  Object.defineProperty(process, 'arch', { value: arch, configurable: true })
}

afterEach(() => {
  stubHost(originalPlatform, originalArch)
})

describe('resolveHostTarget', () => {
  it.each([
    ['darwin', 'arm64', 'aarch64-apple-darwin'],
    ['darwin', 'x64', 'x86_64-apple-darwin'],
    ['linux', 'x64', 'x86_64-unknown-linux-gnu'],
    ['win32', 'x64', 'x86_64-pc-windows-msvc'],
  ])('maps %s/%s to the target triple %s', (platform, arch, expected) => {
    stubHost(platform, arch)

    expect(resolveHostTarget()).toBe(expected)
  })

  it('throws on an unsupported macOS architecture instead of guessing', () => {
    stubHost('darwin', 'ia32')

    expect(() => resolveHostTarget()).toThrow(/unsupported host architecture on macOS/)
  })

  it('throws on an unsupported Linux architecture instead of guessing', () => {
    stubHost('linux', 'arm')

    expect(() => resolveHostTarget()).toThrow(/unsupported host architecture on Linux/)
  })

  it('throws on an unsupported platform instead of guessing', () => {
    stubHost('sunos', 'x64')

    expect(() => resolveHostTarget()).toThrow(/unsupported host platform: sunos/)
  })

  it('returns a value that is always a known target', () => {
    for (const [platform, arch] of [
      ['darwin', 'arm64'],
      ['darwin', 'x64'],
      ['linux', 'x64'],
      ['win32', 'x64'],
    ]) {
      stubHost(platform, arch)
      expect(KNOWN_TARGETS).toContain(resolveHostTarget())
    }
  })
})
