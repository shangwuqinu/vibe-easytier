import type { ConnectionPhase } from '../types'

export interface ConnectionMeta {
  label: string
  tone: 'neutral' | 'info' | 'success' | 'warning' | 'danger'
}

export const connectionMeta = (phase: ConnectionPhase): ConnectionMeta => {
  switch (phase) {
    case 'connected':
      return { label: '已连接', tone: 'success' }
    case 'connecting':
      return { label: '连接中', tone: 'info' }
    case 'disconnecting':
      return { label: '断开中', tone: 'info' }
    case 'recovering':
      return { label: '恢复中', tone: 'warning' }
    case 'failed':
      return { label: '需要处理', tone: 'danger' }
    default:
      return { label: '未连接', tone: 'neutral' }
  }
}

export const canConnect = (phase: ConnectionPhase) =>
  phase === 'disconnected' || phase === 'failed'

export const canDisconnect = (phase: ConnectionPhase) =>
  phase === 'connected' || phase === 'connecting' || phase === 'recovering'

export const formatBytes = (bytes: number) => {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes / 1024
  let unit = 0

  while (value >= 1024 && unit < units.length - 1) {
    value /= 1024
    unit += 1
  }

  return `${value.toFixed(value >= 10 ? 0 : 1)} ${units[unit]}`
}

export const formatBitsPerSecond = (bitsPerSecond: number) => {
  const value = Number.isFinite(bitsPerSecond) ? Math.max(0, bitsPerSecond) : 0
  if (value < 1000) return `${Math.round(value)} bps`
  const units = ['Kbps', 'Mbps', 'Gbps', 'Tbps']
  let scaled = value / 1000
  let unit = 0

  while (scaled >= 1000 && unit < units.length - 1) {
    scaled /= 1000
    unit += 1
  }

  return `${scaled.toFixed(scaled >= 100 ? 0 : scaled >= 10 ? 1 : 2)} ${units[unit]}`
}

export const formatDuration = (startedAt: string | null, now = Date.now()) => {
  if (!startedAt) return '--'
  const elapsedSeconds = Math.max(0, Math.floor((now - Date.parse(startedAt)) / 1000))
  const hours = Math.floor(elapsedSeconds / 3600)
  const minutes = Math.floor((elapsedSeconds % 3600) / 60)
  const seconds = elapsedSeconds % 60

  if (hours > 0) return `${hours} 小时 ${minutes} 分钟`
  if (minutes > 0) return `${minutes} 分钟 ${seconds} 秒`
  return `${seconds} 秒`
}
