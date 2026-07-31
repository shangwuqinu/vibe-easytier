import { describe, expect, it } from 'vitest'

import {
  addBootstrapPeer,
  addBootstrapPeers,
  MAX_BOOTSTRAP_PEERS,
  normalizeBootstrapPeers,
  splitBootstrapPeerInput,
  validateBootstrapPeer,
} from './profile'

describe('bootstrap peer helpers', () => {
  it('trims empty values and preserves the first occurrence of each peer', () => {
    expect(normalizeBootstrapPeers([
      ' tcp://203.0.113.10:11010 ',
      '',
      'tcp://203.0.113.10:11010',
      'udp://203.0.113.20:11010',
    ])).toEqual([
      'tcp://203.0.113.10:11010',
      'udp://203.0.113.20:11010',
    ])
  })

  it('adds a new peer once and leaves an existing peer unchanged', () => {
    const peers = ['tcp://203.0.113.10:11010']

    expect(addBootstrapPeer(peers, 'udp://203.0.113.20:11010')).toEqual([
      'tcp://203.0.113.10:11010',
      'udp://203.0.113.20:11010',
    ])
    expect(addBootstrapPeer(peers, ' tcp://203.0.113.10:11010 ')).toEqual(peers)
  })

  it('splits a pasted multi-protocol peer list and keeps each transport URI', () => {
    const additions = splitBootstrapPeerInput(
      'tcp://203.0.113.10:11010,\n wg://203.0.113.10:11012\nudp://203.0.113.10:11010',
    )

    expect(additions).toEqual([
      'tcp://203.0.113.10:11010',
      'wg://203.0.113.10:11012',
      'udp://203.0.113.10:11010',
    ])
    expect(addBootstrapPeers(['tcp://203.0.113.10:11010'], additions)).toEqual([
      'tcp://203.0.113.10:11010',
      'wg://203.0.113.10:11012',
      'udp://203.0.113.10:11010',
    ])
    expect(MAX_BOOTSTRAP_PEERS).toBe(8)
  })

  it('accepts EasyTier Core wg transport peers with a host and port', () => {
    expect(validateBootstrapPeer('wg://ali.shangwuqiniu.asia:11012')).toBeNull()
    expect(validateBootstrapPeer('wss://seed.example.net:443')).toBeNull()
  })

  it('explains unsupported or incomplete bootstrap peer addresses', () => {
    expect(validateBootstrapPeer('quic://seed.example.net:11010')).toBe('Bootstrap 节点仅支持 tcp://、udp://、wg://、ws:// 或 wss:// 协议。')
    expect(validateBootstrapPeer('wg://')).toBe('Bootstrap 节点必须包含主机地址。')
    expect(validateBootstrapPeer('wg://seed.example.net')).toBe('Bootstrap 节点必须包含端口号，例如 wg://203.0.113.10:11012。')
  })

  it('rejects credentials, query parameters, and fragments like the service', () => {
    const error = 'Bootstrap 节点不可包含账户、密码、查询参数或片段。'

    expect(validateBootstrapPeer('wg://user:password@seed.example.net:11012')).toBe(error)
    expect(validateBootstrapPeer('wg://seed.example.net:11012?token=private')).toBe(error)
    expect(validateBootstrapPeer('wg://seed.example.net:11012#fragment')).toBe(error)
  })
})
