import type { EasyTierFlags } from '../types'

export const EASYTIER_UNLIMITED_BPS = '18446744073709551615'
export const EASYTIER_MAX_FINITE_BPS = '9223372036854775807'
export const EASYTIER_AUTO_QUIC_PORT = 4_294_967_295

export const DEFAULT_EASYTIER_FLAGS: EasyTierFlags = {
  defaultProtocol: 'tcp',
  devName: '',
  enableEncryption: true,
  enableIpv6: true,
  mtu: 1380,
  latencyFirst: false,
  enableExitNode: false,
  noTun: false,
  useSmoltcp: false,
  relayNetworkWhitelist: '*',
  disableP2p: false,
  relayAllPeerRpc: false,
  disableUdpHolePunching: false,
  multiThread: true,
  dataCompressAlgo: 'none',
  bindDevice: true,
  enableKcpProxy: false,
  disableKcpInput: false,
  disableRelayKcp: false,
  proxyForwardBySystem: false,
  acceptDns: false,
  privateMode: true,
  enableQuicProxy: false,
  disableQuicInput: false,
  disableRelayQuic: false,
  quicListenPort: EASYTIER_AUTO_QUIC_PORT,
  foreignRelayBpsLimit: EASYTIER_UNLIMITED_BPS,
  multiThreadCount: 2,
  enableRelayForeignNetworkKcp: false,
  enableRelayForeignNetworkQuic: false,
  encryptionAlgorithm: 'aes-gcm',
  disableSymHolePunching: false,
  tldDnsZone: 'et.net.',
  p2pOnly: false,
  disableTcpHolePunching: false,
  lazyP2p: false,
  needP2p: false,
  instanceRecvBpsLimit: EASYTIER_UNLIMITED_BPS,
  disableUpnp: false,
  disableRelayData: false,
  enableUdpBroadcastRelay: false,
}

export type EasyTierFlagFieldKind = 'toggle' | 'number' | 'text' | 'select' | 'rate'

export interface EasyTierFlagOption {
  value: string
  label: string
}

export interface EasyTierFlagField {
  key: keyof EasyTierFlags
  kind: EasyTierFlagFieldKind
  label: string
  description: string
  options?: EasyTierFlagOption[]
  min?: number
  max?: number
  step?: number
  readOnly?: boolean
}

export interface EasyTierFlagSection {
  id: string
  title: string
  description: string
  fields: EasyTierFlagField[]
}

const protocolOptions: EasyTierFlagOption[] = [
  { value: 'tcp', label: 'TCP' },
  { value: 'udp', label: 'UDP' },
  { value: 'wg', label: 'WireGuard' },
  { value: 'quic', label: 'QUIC' },
  { value: 'ws', label: 'WebSocket' },
  { value: 'wss', label: 'WebSocket TLS' },
  { value: 'faketcp', label: 'FakeTCP' },
]

export const easyTierFlagSections: EasyTierFlagSection[] = [
  {
    id: 'foundation',
    title: '基础与虚拟网卡',
    description: '加密、虚拟网卡、线程与数据处理方式。',
    fields: [
      { key: 'defaultProtocol', kind: 'select', label: '默认传输协议', description: 'Bootstrap 节点未显式写协议时使用的连接协议。', options: protocolOptions },
      { key: 'devName', kind: 'text', label: '虚拟网卡名称', description: '留空时由 EasyTier 管理虚拟网卡名称。' },
      { key: 'enableEncryption', kind: 'toggle', label: '启用加密', description: '加密节点间通信；私有网络通常应保持开启。' },
      { key: 'encryptionAlgorithm', kind: 'select', label: '加密算法', description: '所有节点应使用彼此兼容的加密算法。', options: [
        { value: '', label: '使用 Core 默认' },
        { value: 'aes-gcm', label: 'AES-GCM' },
        { value: 'aes-256-gcm', label: 'AES-256-GCM' },
        { value: 'chacha20', label: 'ChaCha20' },
        { value: 'xor', label: 'XOR（不建议）' },
      ] },
      { key: 'enableIpv6', kind: 'toggle', label: '启用 IPv6', description: '允许 Core 使用 IPv6 底层网络与监听器。' },
      { key: 'mtu', kind: 'number', label: 'MTU', description: '虚拟网卡单包大小。网络出现分片或丢包时可适当降低。', min: 576, max: 65535, step: 1 },
      { key: 'noTun', kind: 'toggle', label: '不创建虚拟网卡', description: '开启后不创建 TUN 虚拟网卡，无法直接使用虚拟局域网地址。' },
      { key: 'useSmoltcp', kind: 'toggle', label: '使用 smoltcp', description: '为子网代理和 KCP 代理使用 smoltcp 协议栈。' },
      { key: 'multiThread', kind: 'toggle', label: '多线程运行', description: '使用多线程运行时处理 Core 工作负载。' },
      { key: 'multiThreadCount', kind: 'number', label: '工作线程数', description: '多线程运行时使用的工作线程数量。', min: 2, max: 128, step: 1 },
      { key: 'bindDevice', kind: 'toggle', label: '绑定物理网卡', description: '将连接套接字绑定到物理网卡，帮助避免路由冲突。' },
      { key: 'dataCompressAlgo', kind: 'select', label: '数据压缩', description: '选择节点间数据压缩方式。', options: [
        { value: 'none', label: '不压缩' },
        { value: 'zstd', label: 'Zstandard' },
      ] },
    ],
  },
  {
    id: 'routing',
    title: '路由与中继',
    description: '转发策略、流量限制和私有网络边界。',
    fields: [
      { key: 'latencyFirst', kind: 'toggle', label: '延迟优先', description: '优先选择延迟更低的路径，而非最短路径。' },
      { key: 'enableExitNode', kind: 'toggle', label: '允许作为出口节点', description: '允许其他节点把全量流量通过本节点转发。' },
      { key: 'proxyForwardBySystem', kind: 'toggle', label: '通过系统转发子网代理', description: '通过系统内核转发子网代理数据包，而非使用内置 NAT。' },
      { key: 'relayNetworkWhitelist', kind: 'text', label: '中继网络白名单', description: '允许转发的网络名称，使用空格分隔；`*` 表示全部。' },
      { key: 'relayAllPeerRpc', kind: 'toggle', label: '转发所有节点 RPC', description: '即使网络不在中继白名单中，也转发节点间控制 RPC。' },
      { key: 'privateMode', kind: 'toggle', label: '私有网络模式', description: '仅允许具有相同网络密钥或受信任凭据的外部网络节点接入。' },
      { key: 'foreignRelayBpsLimit', kind: 'rate', label: '外部网络中继上限', description: '本节点转发其他网络流量的上限，单位 B/s；有限值最大为 9223372036854775807，留空表示无限制。' },
      { key: 'instanceRecvBpsLimit', kind: 'rate', label: '实例接收上限', description: '当前网络实例总接收速率上限，单位 B/s；有限值最大为 9223372036854775807，留空表示无限制。' },
      { key: 'disableRelayData', kind: 'toggle', label: '禁止中继数据', description: '不转发中继数据流，但仍保留控制面连接。' },
    ],
  },
  {
    id: 'p2p',
    title: 'P2P 与 NAT',
    description: '节点直连、打洞和自动端口映射行为。',
    fields: [
      { key: 'disableP2p', kind: 'toggle', label: '禁用自动 P2P', description: '不主动建立普通 P2P 连接；标记为需要 P2P 的节点不受影响。' },
      { key: 'p2pOnly', kind: 'toggle', label: '仅使用 P2P', description: '只与已建立 P2P 连接的节点通信。' },
      { key: 'lazyP2p', kind: 'toggle', label: '按需建立 P2P', description: '仅在实际流量需要时尝试建立 P2P 连接。' },
      { key: 'needP2p', kind: 'toggle', label: '声明需要 P2P', description: '通知其他节点即使启用按需 P2P 也主动连接本节点。' },
      { key: 'disableUdpHolePunching', kind: 'toggle', label: '禁用 UDP 打洞', description: '关闭 UDP NAT 穿透。' },
      { key: 'disableTcpHolePunching', kind: 'toggle', label: '禁用 TCP 打洞', description: '关闭 TCP NAT 穿透。' },
      { key: 'disableSymHolePunching', kind: 'toggle', label: '禁用对称 NAT 打洞', description: '关闭针对对称 NAT 的 UDP 打洞方式。' },
      { key: 'disableUpnp', kind: 'toggle', label: '禁用 UPnP/NAT-PMP', description: '关闭运行时自动端口映射。' },
    ],
  },
  {
    id: 'transport-acceleration',
    title: 'KCP 与 QUIC',
    description: 'TCP 流代理、接入控制和中继策略。',
    fields: [
      { key: 'enableKcpProxy', kind: 'toggle', label: '启用 KCP 代理', description: '将 TCP 流通过 KCP 代理，提高高丢包网络下的表现。' },
      { key: 'disableKcpInput', kind: 'toggle', label: '拒绝 KCP 入站', description: '不允许其他节点通过 KCP 代理访问本节点。' },
      { key: 'disableRelayKcp', kind: 'toggle', label: '禁止中继 KCP', description: '不为其他节点转发 KCP 数据包。' },
      { key: 'enableRelayForeignNetworkKcp', kind: 'toggle', label: '中继外部网络 KCP', description: '允许作为共享节点转发其他网络的 KCP 数据包。' },
      { key: 'enableQuicProxy', kind: 'toggle', label: '启用 QUIC 代理', description: '将 TCP 流通过 QUIC 代理，提高高丢包网络下的表现。' },
      { key: 'disableQuicInput', kind: 'toggle', label: '拒绝 QUIC 入站', description: '不允许其他节点通过 QUIC 代理访问本节点。' },
      { key: 'disableRelayQuic', kind: 'toggle', label: '禁止中继 QUIC', description: '不为其他节点转发 QUIC 数据包。' },
      { key: 'enableRelayForeignNetworkQuic', kind: 'toggle', label: '中继外部网络 QUIC', description: '允许作为共享节点转发其他网络的 QUIC 数据包。' },
      { key: 'quicListenPort', kind: 'number', label: 'QUIC 监听端口（已废弃）', description: '已废弃，修改不会生效；仅用于兼容旧配置，建议保持自动值。', min: 0, max: EASYTIER_AUTO_QUIC_PORT, step: 1, readOnly: true },
    ],
  },
  {
    id: 'dns-windows',
    title: 'DNS 与 Windows',
    description: '系统 DNS 和 Windows 局域网广播转发。',
    fields: [
      { key: 'acceptDns', kind: 'toggle', label: '接受 Magic DNS', description: '允许 EasyTier 修改系统 DNS，以域名访问虚拟网络节点。' },
      { key: 'tldDnsZone', kind: 'text', label: 'Magic DNS 顶级域', description: 'Magic DNS 使用的顶级域，仅在接受 Magic DNS 时生效。' },
      { key: 'enableUdpBroadcastRelay', kind: 'toggle', label: '转发本机 UDP 广播', description: 'Windows 专用；将物理网卡的本机 UDP 广播转发到私有网络，需要管理员权限。' },
    ],
  },
]

export const normalizeEasyTierFlags = (
  flags: Partial<EasyTierFlags> | null | undefined,
): EasyTierFlags => ({
  ...DEFAULT_EASYTIER_FLAGS,
  ...flags,
  foreignRelayBpsLimit: flags?.foreignRelayBpsLimit || EASYTIER_UNLIMITED_BPS,
  instanceRecvBpsLimit: flags?.instanceRecvBpsLimit || EASYTIER_UNLIMITED_BPS,
})

export const rateLimitInputValue = (value: string) =>
  value === EASYTIER_UNLIMITED_BPS ? '' : value

export const rateLimitFromInput = (value: string) =>
  value.trim() || EASYTIER_UNLIMITED_BPS

export const validateEasyTierFlags = (flags: EasyTierFlags) => {
  const errors: Partial<Record<keyof EasyTierFlags, string>> = {}
  const rateKeys: Array<'foreignRelayBpsLimit' | 'instanceRecvBpsLimit'> = [
    'foreignRelayBpsLimit',
    'instanceRecvBpsLimit',
  ]

  if (!Number.isInteger(flags.mtu) || flags.mtu < 576 || flags.mtu > 65535) {
    errors.mtu = 'MTU 必须是 576 到 65535 之间的整数。'
  }
  if (!Number.isInteger(flags.multiThreadCount) || flags.multiThreadCount < 2 || flags.multiThreadCount > 128) {
    errors.multiThreadCount = '工作线程数必须是 2 到 128 之间的整数。'
  }
  if (!Number.isInteger(flags.quicListenPort) || flags.quicListenPort < 0 || flags.quicListenPort > EASYTIER_AUTO_QUIC_PORT) {
    errors.quicListenPort = 'QUIC 监听端口必须是有效的 32 位无符号整数。'
  }
  for (const key of rateKeys) {
    const value = flags[key]
    const isUnlimited = value === EASYTIER_UNLIMITED_BPS
    if (!/^\d+$/.test(value) || (!isUnlimited && BigInt(value) > BigInt(EASYTIER_MAX_FINITE_BPS))) {
      errors[key] = '有限速率必须是 0 到 9223372036854775807 的整数；留空表示无限制。'
    }
  }

  return errors
}
