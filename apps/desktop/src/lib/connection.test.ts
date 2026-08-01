import { describe, expect, it } from 'vitest'

import {
  canConnect,
  canDisconnect,
  connectionMeta,
  formatBitsPerSecond,
  formatBytes,
} from './connection'

describe('connection presentation helpers', () => {
  it('exposes clear commands for each runtime state', () => {
    expect(canConnect('disconnected')).toBe(true)
    expect(canConnect('connecting')).toBe(false)
    expect(canDisconnect('connected')).toBe(true)
    expect(canDisconnect('disconnecting')).toBe(false)
    expect(canDisconnect('failed')).toBe(false)
  })

  it('keeps the runtime state semantic', () => {
    expect(connectionMeta('recovering')).toEqual({
      label: '恢复中',
      tone: 'warning',
    })
    expect(formatBytes(1536)).toBe('1.5 KB')
    expect(formatBitsPerSecond(987)).toBe('987 bps')
    expect(formatBitsPerSecond(12_345_678)).toBe('12.3 Mbps')
  })
})
