import { renderToStaticMarkup } from 'react-dom/server'
import { describe, expect, it } from 'vitest'

import { LogsPage, PeersPage } from './App'
import type { LogEntry, Peer } from './types'

const fullLogLine = [
  'core transport failed for tcp://very-long-bootstrap.example.net:11010/',
  'with a URL/path segment that must wrap instead of being ellipsized',
].join('\n')

const logs: LogEntry[] = [
  {
    id: 'long-log',
    at: '2026-07-31T07:00:00.000Z',
    level: 'error',
    source: 'EasyTier Core',
    message: fullLogLine,
  },
]

const peers: Peer[] = [
  {
    id: 'peer-multi-protocol',
    name: '远端节点',
    hostname: 'remote-node',
    virtualIp: '100.76.1.3',
    protocols: ['wg', 'tcp', 'tcp'],
    role: 'Peer',
    state: 'online',
    latencyMs: 8,
    lastSeen: '2026-07-31T07:00:00.000Z',
    version: '2.6.4',
    sent: 0,
    received: 0,
  },
]

describe('LogsPage', () => {
  it('renders every character and newline of a long log entry', () => {
    const markup = renderToStaticMarkup(<LogsPage logs={logs} onClear={() => undefined} />)

    expect(markup).toContain('log-message')
    expect(markup).toContain(fullLogLine)
  })
})

describe('PeersPage', () => {
  it('renders every active transport for a remote peer as a separate protocol chip', () => {
    const markup = renderToStaticMarkup(
      <PeersPage
        peers={peers}
        onOpenNetwork={() => undefined}
        onRunBandwidthTest={() => Promise.reject(new Error('not run during rendering'))}
      />,
    )

    expect(markup).toContain('连接协议')
    expect(markup).toContain('iperf3 测速')
    expect(markup).toContain('测速')
    expect(markup).toContain('TCP')
    expect(markup).toContain('WireGuard')
    expect((markup.match(/peer-protocol-chip/g) ?? [])).toHaveLength(2)
    expect(markup.indexOf('TCP')).toBeLessThan(markup.indexOf('WireGuard'))
  })
})
