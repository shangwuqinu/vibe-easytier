import { describe, expect, it } from 'vitest'

import {
  DEFAULT_EASYTIER_FLAGS,
  EASYTIER_MAX_FINITE_BPS,
  EASYTIER_UNLIMITED_BPS,
  easyTierFlagSections,
  normalizeEasyTierFlags,
  rateLimitFromInput,
  rateLimitInputValue,
  validateEasyTierFlags,
} from './easytierFlags'

describe('EasyTier flags catalog', () => {
  it('exposes every pinned EasyTier 2.6.4 [flags] key exactly once', () => {
    const catalogKeys = easyTierFlagSections.flatMap((section) => section.fields.map((field) => field.key))
    const defaultKeys = Object.keys(DEFAULT_EASYTIER_FLAGS).sort()

    expect(catalogKeys).toHaveLength(41)
    expect(new Set(catalogKeys).size).toBe(catalogKeys.length)
    expect([...catalogKeys].sort()).toEqual(defaultKeys)
  })

  it('merges older profiles with private encrypted defaults', () => {
    const flags = normalizeEasyTierFlags({ mtu: 1300, latencyFirst: true })

    expect(flags.mtu).toBe(1300)
    expect(flags.latencyFirst).toBe(true)
    expect(flags.privateMode).toBe(true)
    expect(flags.enableEncryption).toBe(true)
  })

  it('round trips an unlimited rate as an empty editor input', () => {
    expect(rateLimitInputValue(EASYTIER_UNLIMITED_BPS)).toBe('')
    expect(rateLimitFromInput('')).toBe(EASYTIER_UNLIMITED_BPS)
    expect(rateLimitFromInput(' 1300 ')).toBe('1300')
  })

  it('retains the deprecated QUIC listen port for imports without offering it as an active setting', () => {
    const quicListenPort = easyTierFlagSections
      .flatMap((section) => section.fields)
      .find((field) => field.key === 'quicListenPort')

    expect(quicListenPort).toMatchObject({
      label: 'QUIC 监听端口（已废弃）',
      readOnly: true,
    })
    expect(quicListenPort?.description).toContain('修改不会生效')
  })

  it('keeps validation strict for numeric values without losing u64 precision', () => {
    expect(validateEasyTierFlags({
      ...DEFAULT_EASYTIER_FLAGS,
      mtu: 575,
      foreignRelayBpsLimit: '9223372036854775808',
    })).toMatchObject({
      mtu: expect.any(String),
      foreignRelayBpsLimit: expect.any(String),
    })

    expect(validateEasyTierFlags({
      ...DEFAULT_EASYTIER_FLAGS,
      foreignRelayBpsLimit: EASYTIER_MAX_FINITE_BPS,
      instanceRecvBpsLimit: EASYTIER_UNLIMITED_BPS,
    })).toEqual({})
  })
})
