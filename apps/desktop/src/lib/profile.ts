export const MAX_BOOTSTRAP_PEERS = 8

export const normalizeBootstrapPeers = (peers: string[]) => {
  const uniquePeers = new Set<string>()

  return peers.reduce<string[]>((normalized, peer) => {
    const value = peer.trim()
    if (!value || uniquePeers.has(value)) return normalized

    uniquePeers.add(value)
    normalized.push(value)
    return normalized
  }, [])
}

export const addBootstrapPeer = (peers: string[], peer: string) =>
  normalizeBootstrapPeers([...peers, peer])

export const splitBootstrapPeerInput = (value: string) =>
  value
    .split(/[\n,]+/)
    .map((peer) => peer.trim())
    .filter(Boolean)

export const addBootstrapPeers = (peers: string[], additions: string[]) =>
  normalizeBootstrapPeers([...peers, ...additions])

const supportedBootstrapProtocols = new Set(['tcp', 'udp', 'wg', 'ws', 'wss'])

const bootstrapAuthority = (value: string) => value.match(/^[a-z][a-z\d+.-]*:\/\/([^/?#]*)/i)?.[1]

const hasExplicitPort = (value: string) => {
  const authority = bootstrapAuthority(value)
  if (!authority) return false

  const hostAndPort = authority.slice(authority.lastIndexOf('@') + 1)
  if (hostAndPort.startsWith('[')) {
    return /^\[[^\]]+\]:\d+$/.test(hostAndPort)
  }

  return /:\d+$/.test(hostAndPort)
}

const hasCredentials = (url: URL, value: string) => {
  const authority = bootstrapAuthority(value)
  const atIndex = authority?.lastIndexOf('@') ?? -1
  if (atIndex < 0) return false

  const userInfo = authority!.slice(0, atIndex)
  return Boolean(url.username || url.password || userInfo.includes(':'))
}

export const validateBootstrapPeer = (peer: string): string | null => {
  const value = peer.trim()
  if (!value) return '请填写 Bootstrap 节点地址。'

  let url: URL
  try {
    url = new URL(value)
  } catch {
    return 'Bootstrap 节点地址格式不正确，请使用完整的协议、主机和端口。'
  }

  const protocol = url.protocol.slice(0, -1).toLowerCase()
  if (!supportedBootstrapProtocols.has(protocol)) {
    return 'Bootstrap 节点仅支持 tcp://、udp://、wg://、ws:// 或 wss:// 协议。'
  }
  if (!url.hostname) return 'Bootstrap 节点必须包含主机地址。'
  if (!hasExplicitPort(value)) {
    return 'Bootstrap 节点必须包含端口号，例如 wg://203.0.113.10:11012。'
  }
  if (hasCredentials(url, value) || value.includes('?') || value.includes('#')) {
    return 'Bootstrap 节点不可包含账户、密码、查询参数或片段。'
  }

  return null
}
