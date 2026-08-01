export type ConnectionPhase =
  | 'disconnected'
  | 'connecting'
  | 'disconnecting'
  | 'connected'
  | 'recovering'
  | 'failed'

export type AppTheme = 'system' | 'light' | 'dark'

export type ServiceHealth = 'healthy' | 'attention' | 'unavailable'

export type PeerState = 'online' | 'offline' | 'relaying'

export type EasyTierDefaultProtocol =
  | 'tcp'
  | 'udp'
  | 'wg'
  | 'quic'
  | 'ws'
  | 'wss'
  | 'faketcp'

export type EasyTierCompressionAlgorithm = 'none' | 'zstd'

export type EasyTierEncryptionAlgorithm =
  | ''
  | 'xor'
  | 'aes-gcm'
  | 'aes-256-gcm'
  | 'chacha20'

/**
 * The complete [flags] surface of the pinned EasyTier Core 2.6.4 runtime.
 * u64 rate values stay decimal strings so a browser never loses precision.
 */
export interface EasyTierFlags {
  defaultProtocol: EasyTierDefaultProtocol
  devName: string
  enableEncryption: boolean
  enableIpv6: boolean
  mtu: number
  latencyFirst: boolean
  enableExitNode: boolean
  noTun: boolean
  useSmoltcp: boolean
  relayNetworkWhitelist: string
  disableP2p: boolean
  relayAllPeerRpc: boolean
  disableUdpHolePunching: boolean
  multiThread: boolean
  dataCompressAlgo: EasyTierCompressionAlgorithm
  bindDevice: boolean
  enableKcpProxy: boolean
  disableKcpInput: boolean
  disableRelayKcp: boolean
  proxyForwardBySystem: boolean
  acceptDns: boolean
  privateMode: boolean
  enableQuicProxy: boolean
  disableQuicInput: boolean
  disableRelayQuic: boolean
  quicListenPort: number
  foreignRelayBpsLimit: string
  multiThreadCount: number
  enableRelayForeignNetworkKcp: boolean
  enableRelayForeignNetworkQuic: boolean
  encryptionAlgorithm: EasyTierEncryptionAlgorithm
  disableSymHolePunching: boolean
  tldDnsZone: string
  p2pOnly: boolean
  disableTcpHolePunching: boolean
  lazyP2p: boolean
  needP2p: boolean
  instanceRecvBpsLimit: string
  disableUpnp: boolean
  disableRelayData: boolean
  enableUdpBroadcastRelay: boolean
}

export interface NetworkProfile {
  id: string
  name: string
  deviceName: string
  networkName: string
  networkSecret: string
  peers: string[]
  virtualIp: string
  flags: EasyTierFlags
  updatedAt: string
}

export interface Peer {
  id: string
  name: string
  hostname: string
  virtualIp: string
  /** Active EasyTier transport protocols reported for this remote peer. */
  protocols: string[]
  role: 'Peer' | 'Relay'
  state: PeerState
  latencyMs: number
  lastSeen: string
  version: string
  sent: number
  received: number
}

export interface BandwidthTestResult {
  peerId: string
  downloadBps: number
  uploadBps: number
  downloadBytes: number
  uploadBytes: number
  durationSeconds: number
  testedAt: string
}

export interface RuntimeState {
  phase: ConnectionPhase
  activeProfileId: string | null
  startedAt: string | null
  retryAt: string | null
  error: string | null
  peerCount: number
  peerCountAvailable: boolean
  lastSuccessAt: string | null
  routes: number
  sent: number
  received: number
  daemonVersion: string
}

export interface Preferences {
  autoConnect: boolean
  serviceAtBoot: boolean
  serviceHealth: ServiceHealth
  theme: AppTheme
}

export interface LogEntry {
  id: string
  at: string
  level: 'info' | 'success' | 'warning' | 'error'
  source: 'Desktop' | 'EasyTier Core' | 'Reconnect'
  message: string
}

export interface DesktopSnapshot {
  profiles: NetworkProfile[]
  peers: Peer[]
  runtime: RuntimeState
  preferences: Preferences
  logs: LogEntry[]
}

export type ProfileDraft = Omit<NetworkProfile, 'id' | 'updatedAt'>
