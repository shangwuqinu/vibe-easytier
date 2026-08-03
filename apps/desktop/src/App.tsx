import {
  Activity,
  ArrowDown,
  ArrowUp,
  Cable,
  Check,
  ChevronRight,
  CircleDot,
  Clock3,
  Copy,
  Edit3,
  Eye,
  EyeOff,
  FileDown,
  FileUp,
  FileText,
  Gauge,
  Globe2,
  LayoutDashboard,
  Link2,
  Menu,
  Maximize2,
  Minus,
  Monitor,
  Network,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Power,
  RefreshCw,
  Search,
  Settings2,
  ShieldCheck,
  SlidersHorizontal,
  Trash2,
  UsersRound,
  Wifi,
  X,
} from 'lucide-react'
import { useEffect, useMemo, useRef, useState } from 'react'
import type { FormEvent, ReactNode } from 'react'

import {
  canConnect,
  canDisconnect,
  connectionMeta,
  formatBitsPerSecond,
  formatBytes,
  formatDuration,
} from './lib/connection'
import {
  DEFAULT_EASYTIER_FLAGS,
  easyTierFlagSections,
  normalizeEasyTierFlags,
  rateLimitFromInput,
  rateLimitInputValue,
  validateEasyTierFlags,
} from './lib/easytierFlags'
import type { EasyTierFlagField } from './lib/easytierFlags'
import { useDesktopState } from './hooks/useDesktopState'
import { getCoreBridge } from './lib/bridge'
import { getErrorMessage } from './lib/error'
import {
  addBootstrapPeers,
  MAX_BOOTSTRAP_PEERS,
  normalizeBootstrapPeers,
  splitBootstrapPeerInput,
  validateBootstrapPeer,
} from './lib/profile'
import type {
  BandwidthTestResult,
  ConnectionPhase,
  EasyTierFlags,
  LogEntry,
  NetworkProfile,
  Preferences,
  ProfileDraft,
} from './types'

type PageId = 'overview' | 'network' | 'peers' | 'logs' | 'settings'

type DialogState =
  | { mode: 'create' }
  | { mode: 'edit'; profile: NetworkProfile }
  | null

const navigation: Array<{
  id: PageId
  label: string
  icon: typeof LayoutDashboard
}> = [
  { id: 'overview', label: '概览', icon: LayoutDashboard },
  { id: 'network', label: '私有网络', icon: Network },
  { id: 'peers', label: '节点', icon: UsersRound },
  { id: 'logs', label: '日志', icon: FileText },
  { id: 'settings', label: '设置', icon: Settings2 },
]

const logLevelLabel: Record<LogEntry['level'], string> = {
  info: '信息',
  success: '成功',
  warning: '警告',
  error: '错误',
}

const serviceHealthLabel: Record<Preferences['serviceHealth'], string> = {
  healthy: '已运行，开机延迟启动',
  attention: '已配置，但当前未运行',
  unavailable: '服务不可用',
}

const logSourceLabel = (source: string) => {
  switch (source) {
    case 'Desktop':
      return '桌面端'
    case 'Reconnect':
      return '自动重连'
    default:
      return source
  }
}

const peerStateLabel = (state: 'online' | 'offline' | 'relaying') => {
  switch (state) {
    case 'online':
      return '在线'
    case 'relaying':
      return '经中继'
    default:
      return '离线'
  }
}

const peerRoleLabel = (role: 'Peer' | 'Relay') => (role === 'Relay' ? '中继' : '节点')

const normalizePeerProtocols = (protocols: readonly string[] | undefined) => {
  const uniqueProtocols = new Set<string>()

  for (const protocol of protocols ?? []) {
    const normalized = protocol.trim().toLowerCase()
    if (normalized) uniqueProtocols.add(normalized)
  }

  // EasyTier collects live tunnels from a concurrent map, so their source
  // order may change between status polls. Keep the node table visually
  // stable while still rendering every active transport.
  return [...uniqueProtocols].sort((left, right) => left.localeCompare(right, 'en'))
}

const peerProtocolLabel = (protocol: string) => {
  switch (protocol) {
    case 'wg':
      return 'WireGuard'
    case 'faketcp':
      return 'FakeTCP'
    default:
      return protocol.toUpperCase()
  }
}

const blankProfile: ProfileDraft = {
  name: '',
  deviceName: '',
  networkName: '',
  networkSecret: '',
  peers: [],
  virtualIp: '',
  flags: { ...DEFAULT_EASYTIER_FLAGS },
}

const profileToDraft = (profile: NetworkProfile): ProfileDraft => ({
  name: profile.name,
  deviceName: profile.deviceName,
  networkName: profile.networkName,
  networkSecret: profile.networkSecret,
  peers: profile.peers,
  virtualIp: profile.virtualIp,
  flags: normalizeEasyTierFlags(profile.flags),
})

const formatTimestamp = (value: string) =>
  new Intl.DateTimeFormat('zh-CN', {
    hour: '2-digit',
    minute: '2-digit',
    second: '2-digit',
  }).format(new Date(value))

const formatDateTime = (value: string) =>
  new Intl.DateTimeFormat('zh-CN', {
    month: 'short',
    day: 'numeric',
    hour: '2-digit',
    minute: '2-digit',
  }).format(new Date(value))

const isIpv4Cidr = (value: string) => {
  const [address, prefix, ...extra] = value.split('/')
  if (extra.length || !address || !prefix || !/^\d{1,2}$/.test(prefix)) return false
  const prefixLength = Number(prefix)
  if (prefixLength < 1 || prefixLength > 32) return false
  const octets = address.split('.')
  return octets.length === 4 && octets.every((octet) => /^\d{1,3}$/.test(octet) && Number(octet) <= 255)
}

const useErrorFocus = (error: string) => {
  const errorRef = useRef<HTMLParagraphElement | null>(null)

  useEffect(() => {
    if (!error) return

    const frame = window.requestAnimationFrame(() => {
      errorRef.current?.scrollIntoView({ behavior: 'smooth', block: 'nearest' })
      errorRef.current?.focus({ preventScroll: true })
    })
    return () => window.cancelAnimationFrame(frame)
  }, [error])

  return errorRef
}

export const App = () => {
  const desktop = useDesktopState()
  const [page, setPage] = useState<PageId>('overview')
  const [navCollapsed, setNavCollapsed] = useState(false)
  const [mobileNavOpen, setMobileNavOpen] = useState(false)
  const [dialog, setDialog] = useState<DialogState>(null)
  const pageContentRef = useRef<HTMLElement | null>(null)

  const activeProfile = useMemo(
    () =>
      desktop.snapshot.profiles.find(
        (profile) => profile.id === desktop.snapshot.runtime.activeProfileId,
      ) ?? null,
    [desktop.snapshot.profiles, desktop.snapshot.runtime.activeProfileId],
  )

  const selectPage = (nextPage: PageId) => {
    setPage(nextPage)
    setMobileNavOpen(false)
  }

  useEffect(() => {
    pageContentRef.current?.scrollTo({ top: 0, left: 0 })
  }, [page])

  const controlWindow = (action: 'minimizeWindow' | 'toggleMaximizeWindow' | 'hideWindow') => {
    void getCoreBridge()?.[action]?.()
  }

  const renderPage = () => {
    switch (page) {
      case 'network':
        return (
          <PrivateNetworkPage
            profiles={desktop.snapshot.profiles}
            activeProfile={activeProfile}
            phase={desktop.snapshot.runtime.phase}
            onCreate={() => setDialog({ mode: 'create' })}
            onImport={() => desktop.importProfile()}
            onExport={(profileId) => desktop.exportProfile(profileId)}
            onEdit={(profile) => setDialog({ mode: 'edit', profile })}
            onDelete={(profileId) => void desktop.deleteProfile(profileId)}
            onSelect={desktop.selectProfile}
            onConnect={() => void desktop.connect()}
            onDisconnect={() => void desktop.disconnect()}
          />
        )
      case 'peers':
        return (
          <PeersPage
            peers={desktop.snapshot.peers}
            onOpenNetwork={() => selectPage('network')}
            onRunBandwidthTest={desktop.runBandwidthTest}
          />
        )
      case 'logs':
        return <LogsPage logs={desktop.snapshot.logs} onClear={() => void desktop.clearLogs()} />
      case 'settings':
        return (
          <SettingsPage
            preferences={desktop.snapshot.preferences}
            isNative={desktop.isNative}
            coreVersion={desktop.snapshot.runtime.daemonVersion}
            onSetAutoConnect={(enabled) => void desktop.setAutoConnect(enabled)}
            onSetTheme={(theme) => void desktop.setTheme(theme)}
            activeProfile={activeProfile}
            onSaveFlags={desktop.updateProfileFlags}
            onOpenNetwork={() => selectPage('network')}
          />
        )
      default:
        return (
          <OverviewPage
            activeProfile={activeProfile}
            phase={desktop.snapshot.runtime.phase}
            startedAt={desktop.snapshot.runtime.startedAt}
            routes={desktop.snapshot.runtime.routes}
            sent={desktop.snapshot.runtime.sent}
            received={desktop.snapshot.runtime.received}
            peerCount={desktop.snapshot.runtime.peerCount}
            peerCountAvailable={desktop.snapshot.runtime.peerCountAvailable}
            lastSuccessAt={desktop.snapshot.runtime.lastSuccessAt}
            retryAt={desktop.snapshot.runtime.retryAt}
            error={desktop.snapshot.runtime.error}
            peers={desktop.snapshot.peers}
            preferences={desktop.snapshot.preferences}
            logs={desktop.snapshot.logs}
            onConnect={() => void desktop.connect()}
            onDisconnect={() => void desktop.disconnect()}
            onSetAutoConnect={(enabled) => void desktop.setAutoConnect(enabled)}
            onOpenNetwork={() => selectPage('network')}
          />
        )
    }
  }

  return (
    <div className={`app-shell ${navCollapsed ? 'app-shell--compact' : ''}`}>
      <aside className="sidebar" aria-label="应用导航">
        <div className="brand-row">
          <button
            className="brand"
            type="button"
            onClick={() => selectPage('overview')}
            aria-label="Vibe EasyTier 概览"
          >
            <span className="brand-mark"><Network size={20} strokeWidth={2.4} /></span>
            <span className="brand-name">Vibe EasyTier</span>
          </button>
          <IconButton
            className="collapse-control"
            title={navCollapsed ? '展开导航' : '收起导航'}
            onClick={() => setNavCollapsed((collapsed) => !collapsed)}
          >
            {navCollapsed ? <PanelLeftOpen size={18} /> : <PanelLeftClose size={18} />}
          </IconButton>
        </div>

        <nav className="side-nav">
          {navigation.map((item) => {
            const Icon = item.icon
            const selected = page === item.id
            return (
              <button
                key={item.id}
                className={`nav-item ${selected ? 'nav-item--active' : ''}`}
                type="button"
                title={navCollapsed ? item.label : undefined}
                onClick={() => selectPage(item.id)}
              >
                <Icon size={19} strokeWidth={selected ? 2.4 : 2} />
                <span>{item.label}</span>
              </button>
            )
          })}
        </nav>

        <div className="sidebar-footer">
          <div className="side-runtime">
            <StatusDot phase={desktop.snapshot.runtime.phase} />
            <div>
              <strong>{connectionMeta(desktop.snapshot.runtime.phase).label}</strong>
              <span>{activeProfile?.name ?? '未选择档案'}</span>
            </div>
          </div>
          <div className="side-transfer">
            <span><ArrowUp size={13} /> {formatBytes(desktop.snapshot.runtime.sent)}</span>
            <span><ArrowDown size={13} /> {formatBytes(desktop.snapshot.runtime.received)}</span>
          </div>
        </div>
      </aside>

      <div className="workbench">
        <header className="top-bar">
          <button
            className="mobile-menu"
            type="button"
            title="打开导航"
            onClick={() => setMobileNavOpen(true)}
          >
            <Menu size={20} />
          </button>
          <div className="mobile-brand"><Network size={18} /><span>Vibe EasyTier</span></div>
          <div className="window-drag-region" data-tauri-drag-region />
          <div className="top-bar-status">
            <StatusBadge phase={desktop.snapshot.runtime.phase} />
            {activeProfile && <span className="top-profile-name">{activeProfile.name}</span>}
          </div>
          {desktop.isNative && (
            <div className="window-controls" aria-label="窗口控制">
              <IconButton title="最小化窗口" onClick={() => controlWindow('minimizeWindow')}><Minus size={16} /></IconButton>
              <IconButton title="最大化或还原窗口" onClick={() => controlWindow('toggleMaximizeWindow')}><Maximize2 size={15} /></IconButton>
              <IconButton className="window-controls__close" title="关闭到托盘" onClick={() => controlWindow('hideWindow')}><X size={16} /></IconButton>
            </div>
          )}
        </header>

        <main className="page-content" ref={pageContentRef}>{renderPage()}</main>
      </div>

      {mobileNavOpen && (
        <>
          <div className="mobile-nav-backdrop mobile-nav-backdrop--open" onClick={() => setMobileNavOpen(false)} />
          <aside className="mobile-nav mobile-nav--open">
            <div className="mobile-nav-header">
              <div className="brand"><span className="brand-mark"><Network size={20} /></span><span>Vibe EasyTier</span></div>
              <IconButton title="关闭导航" onClick={() => setMobileNavOpen(false)}>
                <X size={18} />
              </IconButton>
            </div>
            <nav className="side-nav">
              {navigation.map((item) => {
                const Icon = item.icon
                return (
                  <button
                    key={item.id}
                    className={`nav-item ${page === item.id ? 'nav-item--active' : ''}`}
                    type="button"
                    onClick={() => selectPage(item.id)}
                  >
                    <Icon size={19} />
                    <span>{item.label}</span>
                  </button>
                )
              })}
            </nav>
          </aside>
        </>
      )}

      <nav className="bottom-nav" aria-label="移动端导航">
        {navigation.map((item) => {
          const Icon = item.icon
          return (
            <button
              key={item.id}
              type="button"
              className={page === item.id ? 'bottom-nav__item bottom-nav__item--active' : 'bottom-nav__item'}
              onClick={() => selectPage(item.id)}
            >
              <Icon size={18} />
              <span>{item.label}</span>
            </button>
          )
        })}
      </nav>

      {dialog && (
        <ProfileDialog
          profile={dialog.mode === 'edit' ? dialog.profile : null}
          onClose={() => setDialog(null)}
          onSave={async (draft) => {
            await desktop.saveProfile(draft, dialog.mode === 'edit' ? dialog.profile.id : undefined)
            setDialog(null)
          }}
        />
      )}
    </div>
  )
}

const PageHeader = ({
  title,
  actions,
}: {
  title: string
  actions?: ReactNode
}) => (
  <header className="page-header">
    <h1>{title}</h1>
    {actions && <div className="page-actions">{actions}</div>}
  </header>
)

const IconButton = ({
  children,
  className = '',
  title,
  onClick,
  disabled = false,
}: {
  children: ReactNode
  className?: string
  title: string
  onClick: () => void
  disabled?: boolean
}) => (
  <button
    className={`icon-button ${className}`}
    type="button"
    title={title}
    aria-label={title}
    onClick={onClick}
    disabled={disabled}
  >
    {children}
  </button>
)

const StatusDot = ({ phase }: { phase: ConnectionPhase }) => (
  <span className={`status-dot status-dot--${connectionMeta(phase).tone}`} aria-hidden="true" />
)

const StatusBadge = ({ phase }: { phase: ConnectionPhase }) => {
  const meta = connectionMeta(phase)
  return (
    <span className={`status-badge status-badge--${meta.tone}`}>
      <StatusDot phase={phase} />
      {meta.label}
    </span>
  )
}

const Toggle = ({
  checked,
  onChange,
  label,
  disabled = false,
}: {
  checked: boolean
  onChange: (checked: boolean) => void
  label: string
  disabled?: boolean
}) => (
  <button
    className={`toggle ${checked ? 'toggle--checked' : ''}`}
    type="button"
    role="switch"
    aria-checked={checked}
    aria-label={label}
    title={label}
    disabled={disabled}
    onClick={() => onChange(!checked)}
  >
    <span />
  </button>
)

const Panel = ({
  title,
  icon,
  tone = 'blue',
  action,
  children,
  className = '',
}: {
  title: string
  icon: ReactNode
  tone?: 'blue' | 'orange' | 'green' | 'red' | 'slate'
  action?: ReactNode
  children: ReactNode
  className?: string
}) => (
  <section className={`panel ${className}`}>
    <header className="panel__header">
      <div className="panel__heading">
        <span className={`panel__icon panel__icon--${tone}`}>{icon}</span>
        <h2>{title}</h2>
      </div>
      {action && <div className="panel__action">{action}</div>}
    </header>
    <div className="panel__body">{children}</div>
  </section>
)

const MetricCard = ({
  label,
  value,
  icon,
  tone,
}: {
  label: string
  value: string | number
  icon: ReactNode
  tone: 'blue' | 'orange' | 'green' | 'slate'
}) => (
  <article className="metric-card">
    <span className={`metric-card__icon metric-card__icon--${tone}`}>{icon}</span>
    <div>
      <span className="metric-card__label">{label}</span>
      <strong className="metric-card__value">{value}</strong>
    </div>
  </article>
)

const OverviewPage = ({
  activeProfile,
  phase,
  startedAt,
  routes,
  sent,
  received,
  peerCount,
  peerCountAvailable,
  lastSuccessAt,
  retryAt,
  error,
  peers,
  preferences,
  logs,
  onConnect,
  onDisconnect,
  onSetAutoConnect,
  onOpenNetwork,
}: {
  activeProfile: NetworkProfile | null
  phase: ConnectionPhase
  startedAt: string | null
  routes: number
  sent: number
  received: number
  peerCount: number
  peerCountAvailable: boolean
  lastSuccessAt: string | null
  retryAt: string | null
  error: string | null
  peers: ReturnType<typeof useDesktopState>['snapshot']['peers']
  preferences: Preferences
  logs: LogEntry[]
  onConnect: () => void
  onDisconnect: () => void
  onSetAutoConnect: (enabled: boolean) => void
  onOpenNetwork: () => void
}) => {
  const onlinePeers = peers.filter((peer) => peer.state !== 'offline')
  const displayedPeerCount = peerCountAvailable ? peerCount : onlinePeers.length
  const transitional =
    phase === 'connecting' || phase === 'recovering' || phase === 'disconnecting'

  return (
    <>
      <PageHeader title="概览" />

      <section className="connection-console" aria-label="连接控制">
        <div className="connection-console__status">
          <div className="connection-pulse"><StatusDot phase={phase} /></div>
          <div>
            <StatusBadge phase={phase} />
            <h2>{activeProfile?.name ?? '未选择私有网络'}</h2>
            <span>{activeProfile?.networkName ?? '请选择一个私有网络档案'}</span>
          </div>
        </div>
        <div className="connection-console__actions">
          {canDisconnect(phase) ? (
            <button className="button button--quiet" type="button" onClick={onDisconnect} disabled={transitional}>
              <Power size={17} /> 断开连接
            </button>
          ) : (
            <button className="button button--primary" type="button" onClick={onConnect} disabled={!activeProfile || !canConnect(phase) || transitional}>
              <Power size={17} /> 连接
            </button>
          )}
          <button className="button button--secondary" type="button" onClick={onOpenNetwork}>
            <SlidersHorizontal size={17} /> 网络配置
          </button>
        </div>
      </section>

      <section className="metric-grid" aria-label="实时指标">
        <MetricCard label="在线节点" value={peerCountAvailable ? displayedPeerCount : '--'} icon={<UsersRound size={20} />} tone="green" />
        <MetricCard label="路由" value={routes} icon={<Cable size={20} />} tone="blue" />
        <MetricCard label="已发送" value={formatBytes(sent)} icon={<ArrowUp size={20} />} tone="orange" />
        <MetricCard label="已接收" value={formatBytes(received)} icon={<ArrowDown size={20} />} tone="blue" />
      </section>

      <section className="dashboard-grid">
        <Panel
          title="私有网络"
          icon={<ShieldCheck size={20} />}
          tone="green"
          action={<IconButton title="打开私有网络" onClick={onOpenNetwork}><ChevronRight size={18} /></IconButton>}
        >
          {activeProfile ? (
            <div className="detail-list">
              <Detail label="网络名称" value={activeProfile.networkName} />
              <Detail label="虚拟 IPv4" value={activeProfile.virtualIp || '--'} mono />
              <Detail label="Bootstrap 节点" value={String(activeProfile.peers.length)} />
              <Detail label="运行时长" value={phase === 'connected' ? formatDuration(startedAt) : '--'} />
            </div>
          ) : (
            <EmptyState icon={<Network size={26} />} label="暂无私有网络档案" actionLabel="新建档案" onAction={onOpenNetwork} />
          )}
        </Panel>

        <Panel title="启动与连接" icon={<RefreshCw size={20} />} tone="blue">
          <div className="setting-list setting-list--compact">
            <SettingRow label="Windows 开机服务" value={serviceHealthLabel[preferences.serviceHealth]}>{null}</SettingRow>
            <SettingRow label="自动连接" value={preferences.autoConnect ? '已开启' : '已关闭'}>
              <Toggle checked={preferences.autoConnect} onChange={onSetAutoConnect} label="自动连接" />
            </SettingRow>
            <SettingRow label="最近核心健康检查" value={lastSuccessAt ? formatDateTime(lastSuccessAt) : '--'}>{null}</SettingRow>
            {retryAt && <SettingRow label="下次重试" value={formatDateTime(retryAt)}>{null}</SettingRow>}
            {error && <SettingRow label="连接错误" value={error}>{null}</SettingRow>}
          </div>
        </Panel>

        <Panel title="在线节点" icon={<Wifi size={20} />} tone="orange" className="panel--wide">
          {peers.length ? (
            <div className="overview-peer-list">
              {peers.slice(0, 3).map((peer) => (
                <div className="overview-peer" key={peer.id}>
                  <span className={`peer-avatar peer-avatar--${peer.state}`}><CircleDot size={16} /></span>
                  <div>
                    <strong>{peer.name}</strong>
                    <span>{peer.virtualIp}</span>
                  </div>
                  <span className="peer-latency">{peer.state === 'offline' ? '离线' : `${peer.latencyMs} ms`}</span>
                </div>
              ))}
            </div>
          ) : (
            <EmptyState icon={<UsersRound size={26} />} label="暂无节点状态" actionLabel="打开私有网络" onAction={onOpenNetwork} />
          )}
        </Panel>

        <Panel title="活动" icon={<Activity size={20} />} tone="slate" className="panel--wide">
          <div className="activity-list">
            {logs.slice(0, 4).map((log) => (
              <div className="activity-item" key={log.id}>
                <span className={`log-marker log-marker--${log.level}`} />
                <div><strong>{log.message}</strong><span>{logSourceLabel(log.source)}</span></div>
                <time>{formatTimestamp(log.at)}</time>
              </div>
            ))}
          </div>
        </Panel>
      </section>
    </>
  )
}

const Detail = ({
  label,
  value,
  mono = false,
}: {
  label: string
  value: string
  mono?: boolean
}) => (
  <div className="detail-row">
    <span>{label}</span>
    <strong className={mono ? 'mono' : ''}>{value}</strong>
  </div>
)

const SettingRow = ({
  label,
  value,
  children,
}: {
  label: string
  value?: string
  children: ReactNode
}) => (
  <div className="setting-row">
    <div><strong>{label}</strong>{value && <span>{value}</span>}</div>
    {children}
  </div>
)

const EmptyState = ({
  icon,
  label,
  actionLabel,
  onAction,
}: {
  icon: ReactNode
  label: string
  actionLabel: string
  onAction: () => void
}) => (
  <div className="empty-state">
    <span>{icon}</span>
    <strong>{label}</strong>
    <button className="button button--secondary" type="button" onClick={onAction}>{actionLabel}</button>
  </div>
)

const PrivateNetworkPage = ({
  profiles,
  activeProfile,
  phase,
  onCreate,
  onImport,
  onExport,
  onEdit,
  onDelete,
  onSelect,
  onConnect,
  onDisconnect,
}: {
  profiles: NetworkProfile[]
  activeProfile: NetworkProfile | null
  phase: ConnectionPhase
  onCreate: () => void
  onImport: () => Promise<NetworkProfile | null>
  onExport: (profileId: string) => Promise<string | null>
  onEdit: (profile: NetworkProfile) => void
  onDelete: (id: string) => void
  onSelect: (id: string) => void
  onConnect: () => void
  onDisconnect: () => void
}) => {
  const [copied, setCopied] = useState(false)
  const [importing, setImporting] = useState(false)
  const [importError, setImportError] = useState('')
  const importErrorRef = useErrorFocus(importError)

  const copyAddress = async () => {
    if (!activeProfile?.virtualIp) return
    try {
      await navigator.clipboard.writeText(activeProfile.virtualIp)
      setCopied(true)
      window.setTimeout(() => setCopied(false), 1200)
    } catch {
      setCopied(false)
    }
  }

  const importLocalToml = async () => {
    if (importing) return
    setImporting(true)
    setImportError('')
    try {
      await onImport()
    } catch (error) {
      setImportError(getErrorMessage(error, '无法从本地文件导入 TOML。'))
    } finally {
      setImporting(false)
    }
  }

  return (
    <>
      <PageHeader
        title="私有网络"
        actions={
          <>
            <button
              className="button button--secondary"
              type="button"
              title="选择本地 .toml 文件并导入"
              onClick={() => void importLocalToml()}
              disabled={importing}
            >
              <FileUp size={17} /> {importing ? '正在导入' : '导入本地 TOML'}
            </button>
            <button
              className="button button--secondary"
              type="button"
              title="导出完整 TOML，文件中包含网络密钥"
              onClick={() => {
                if (activeProfile) void onExport(activeProfile.id).catch(() => undefined)
              }}
              disabled={!activeProfile}
            >
              <FileDown size={17} /> 导出 TOML
            </button>
            <button className="button button--primary" type="button" onClick={onCreate}><Plus size={17} /> 新建档案</button>
          </>
        }
      />
      {importError && <p className="form-error" ref={importErrorRef} tabIndex={-1} role="alert">导入档案失败：{importError}</p>}
      <section className="network-workspace">
        <div className="profile-rail" aria-label="私有网络档案">
          {profiles.map((profile) => {
            const selected = profile.id === activeProfile?.id
            return (
              <button
                className={`profile-tile ${selected ? 'profile-tile--active' : ''}`}
                type="button"
                key={profile.id}
                onClick={() => onSelect(profile.id)}
              >
                <span className="profile-tile__mark"><Network size={18} /></span>
                <span className="profile-tile__content">
                  <strong>{profile.name}</strong>
                  <span>{profile.networkName}</span>
                </span>
                {selected && <Check size={16} />}
              </button>
            )
          })}
        </div>

        <Panel
          title={activeProfile?.name ?? '私有网络'}
          icon={<Network size={20} />}
          tone="blue"
          className="network-detail"
          action={
            activeProfile ? (
              <div className="panel-icon-actions">
                <IconButton title="编辑档案" onClick={() => onEdit(activeProfile)}><Edit3 size={17} /></IconButton>
                <IconButton title="删除档案" onClick={() => onDelete(activeProfile.id)}><Trash2 size={17} /></IconButton>
              </div>
            ) : undefined
          }
        >
          {activeProfile ? (
            <div className="network-detail__body">
              <div className="network-state-line">
                <StatusBadge phase={phase} />
                {canDisconnect(phase) ? (
                  <button className="button button--quiet" type="button" onClick={onDisconnect}><Power size={17} /> 断开连接</button>
                ) : (
                  <button className="button button--primary" type="button" onClick={onConnect} disabled={!canConnect(phase)}><Power size={17} /> 连接</button>
                )}
              </div>
              <div className="network-data-grid">
                <div className="data-field"><span>网络名称</span><strong>{activeProfile.networkName}</strong></div>
                <div className="data-field"><span>设备名称</span><strong>{activeProfile.deviceName || 'Windows 本机名称（自动）'}</strong></div>
                <div className="data-field"><span>虚拟 IPv4</span><strong className="mono">{activeProfile.virtualIp || '--'}</strong><IconButton title={copied ? '已复制' : '复制虚拟 IPv4'} onClick={() => void copyAddress()}><Copy size={15} /></IconButton></div>
                <div className="data-field data-field--wide"><span>网络密钥</span><strong className="mono">••••••••••••••••</strong></div>
                <div className="data-field data-field--wide"><span>Bootstrap 节点</span><div className="endpoint-list">{activeProfile.peers.map((peer) => <code key={peer}>{peer}</code>)}</div></div>
              </div>
            </div>
          ) : (
            <EmptyState icon={<Network size={26} />} label="暂无私有网络档案" actionLabel="新建档案" onAction={onCreate} />
          )}
        </Panel>
      </section>
    </>
  )
}

const PeerProtocolList = ({ protocols }: { protocols?: readonly string[] }) => {
  const visibleProtocols = normalizePeerProtocols(protocols)

  if (!visibleProtocols.length) return <span className="peer-protocol-empty">--</span>

  return (
    <div className="peer-protocol-list" aria-label={`连接协议：${visibleProtocols.map(peerProtocolLabel).join('、')}`}>
      {visibleProtocols.map((protocol) => (
        <span className="peer-protocol-chip" key={protocol} title={`EasyTier ${peerProtocolLabel(protocol)} 传输`}>
          {peerProtocolLabel(protocol)}
        </span>
      ))}
    </div>
  )
}

export const PeersPage = ({
  peers,
  onOpenNetwork,
  onRunBandwidthTest,
}: {
  peers: ReturnType<typeof useDesktopState>['snapshot']['peers']
  onOpenNetwork: () => void
  onRunBandwidthTest: (peerId: string) => Promise<BandwidthTestResult>
}) => {
  const [bandwidthTests, setBandwidthTests] = useState<Record<string, {
    status: 'running' | 'success' | 'error'
    result?: BandwidthTestResult
    error?: string
  }>>({})
  const runningTestRef = useRef(false)
  const online = peers.filter((peer) => peer.state !== 'offline').length
  const testRunning = Object.values(bandwidthTests).some((test) => test.status === 'running')

  const runTest = async (peerId: string) => {
    if (runningTestRef.current) return
    runningTestRef.current = true
    setBandwidthTests((current) => ({
      ...current,
      [peerId]: { status: 'running' },
    }))
    try {
      const result = await onRunBandwidthTest(peerId)
      setBandwidthTests((current) => ({
        ...current,
        [peerId]: { status: 'success', result },
      }))
    } catch (error) {
      setBandwidthTests((current) => ({
        ...current,
        [peerId]: {
          status: 'error',
          error: getErrorMessage(error, '节点间带宽测试失败。'),
        },
      }))
    } finally {
      runningTestRef.current = false
    }
  }

  return (
    <>
      <PageHeader
        title="节点"
        actions={<span className="table-summary"><span className="status-dot status-dot--success" />{online} 个在线</span>}
      />
      <section className="peer-table-wrap">
        {peers.length ? (
          <table className="peer-table">
            <thead>
              <tr><th>节点</th><th>地址</th><th>角色</th><th>状态</th><th>连接协议</th><th>延迟</th><th>版本</th><th>最近出现</th><th>iperf3 测速</th></tr>
            </thead>
            <tbody>
              {peers.map((peer) => {
                const test = bandwidthTests[peer.id]
                return (
                  <tr key={peer.id}>
                    <td data-label="节点"><div className="peer-name"><span className={`peer-avatar peer-avatar--${peer.state}`}><CircleDot size={16} /></span><div><strong>{peer.name}</strong><span>{peer.hostname}</span></div></div></td>
                    <td data-label="地址"><code>{peer.virtualIp}</code></td>
                    <td data-label="角色"><span className="role-chip">{peerRoleLabel(peer.role)}</span></td>
                    <td data-label="状态"><span className={`peer-state peer-state--${peer.state}`}><span />{peerStateLabel(peer.state)}</span></td>
                    <td data-label="连接协议"><PeerProtocolList protocols={peer.protocols} /></td>
                    <td data-label="延迟">{peer.state === 'offline' ? '--' : `${peer.latencyMs} ms`}</td>
                    <td data-label="版本">{peer.version}</td>
                    <td data-label="最近出现">{formatDateTime(peer.lastSeen)}</td>
                    <td data-label="iperf3 测速">
                      {test?.status === 'success' && test.result ? (
                        <div className="bandwidth-result">
                          <span><ArrowDown size={13} />{formatBitsPerSecond(test.result.downloadBps)}</span>
                          <span><ArrowUp size={13} />{formatBitsPerSecond(test.result.uploadBps)}</span>
                          <IconButton title="重新运行 iperf3 测速" onClick={() => void runTest(peer.id)} disabled={testRunning}>
                            <RefreshCw size={14} />
                          </IconButton>
                        </div>
                      ) : test?.status === 'error' ? (
                        <div className="bandwidth-error">
                          <span>{test.error}</span>
                          <IconButton title="重试 iperf3 测速" onClick={() => void runTest(peer.id)} disabled={testRunning}>
                            <RefreshCw size={14} />
                          </IconButton>
                        </div>
                      ) : (
                        <button
                          className="button button--secondary bandwidth-button"
                          type="button"
                          onClick={() => void runTest(peer.id)}
                          disabled={peer.state === 'offline' || testRunning}
                        >
                          {test?.status === 'running' ? <RefreshCw className="spin" size={15} /> : <Gauge size={15} />}
                          {test?.status === 'running' ? '测速中' : '测速'}
                        </button>
                      )}
                    </td>
                  </tr>
                )
              })}
            </tbody>
          </table>
        ) : (
          <EmptyState icon={<UsersRound size={26} />} label="暂无节点状态" actionLabel="打开私有网络" onAction={onOpenNetwork} />
        )}
      </section>
    </>
  )
}

export const LogsPage = ({ logs, onClear }: { logs: LogEntry[]; onClear: () => void }) => {
  const [query, setQuery] = useState('')
  const filteredLogs = useMemo(() => {
    const normalized = query.trim().toLowerCase()
    if (!normalized) return logs
    return logs.filter((log) => `${log.message} ${log.source} ${log.level}`.toLowerCase().includes(normalized))
  }, [logs, query])

  return (
    <>
      <PageHeader
        title="日志"
        actions={
          <>
            <label className="search-field">
              <Search size={16} />
              <input value={query} onChange={(event) => setQuery(event.target.value)} placeholder="搜索日志" aria-label="搜索日志" />
            </label>
            <IconButton title="清空日志" onClick={onClear} disabled={logs.length === 0}><Trash2 size={17} /></IconButton>
          </>
        }
      />
      <div className="logs-page">
        <section className="logs-surface" aria-label="日志内容">
          {filteredLogs.length ? (
            <div className="log-list">
              {filteredLogs.map((log) => (
                <article className="log-row" key={log.id}>
                  <span className={`log-marker log-marker--${log.level}`} />
                  <time>{formatTimestamp(log.at)}</time>
                  <span className="log-source">{logSourceLabel(log.source)}</span>
                  <p className="log-message">{log.message}</p>
                  <span className={`log-level log-level--${log.level}`}>{logLevelLabel[log.level]}</span>
                </article>
              ))}
            </div>
          ) : (
            <EmptyState icon={<FileText size={26} />} label="没有匹配的日志" actionLabel="清除搜索" onAction={() => setQuery('')} />
          )}
        </section>
      </div>
    </>
  )
}

const SettingsPage = ({
  preferences,
  isNative,
  coreVersion,
  onSetAutoConnect,
  onSetTheme,
  activeProfile,
  onSaveFlags,
  onOpenNetwork,
}: {
  preferences: Preferences
  isNative: boolean
  coreVersion: string
  onSetAutoConnect: (enabled: boolean) => void
  onSetTheme: (theme: Preferences['theme']) => void
  activeProfile: NetworkProfile | null
  onSaveFlags: (profileId: string, flags: EasyTierFlags) => Promise<NetworkProfile>
  onOpenNetwork: () => void
}) => (
  <>
    <PageHeader title="设置" />
    <section className="settings-grid">
      <Panel title="启动" icon={<Power size={20} />} tone="green">
        <div className="setting-list">
          <SettingRow label="Windows 开机服务" value={preferences.serviceAtBoot ? '已配置延迟自动启动' : '服务不可用'}>{null}</SettingRow>
          <SettingRow label="服务健康状态" value={serviceHealthLabel[preferences.serviceHealth]}>{null}</SettingRow>
          <SettingRow label="自动连接" value={preferences.autoConnect ? '已开启' : '已关闭'}>
            <Toggle checked={preferences.autoConnect} onChange={onSetAutoConnect} label="自动连接" />
          </SettingRow>
        </div>
      </Panel>
      <Panel title="外观" icon={<Monitor size={20} />} tone="blue">
        <div className="setting-list">
          <div className="setting-row">
            <div><strong>颜色模式</strong><span>{preferences.theme === 'system' ? '跟随系统' : preferences.theme === 'dark' ? '深色' : '浅色'}</span></div>
            <div className="segmented-control" aria-label="颜色模式">
              {(['system', 'light', 'dark'] as const).map((theme) => (
                <button key={theme} type="button" className={preferences.theme === theme ? 'segmented-control__item segmented-control__item--active' : 'segmented-control__item'} onClick={() => onSetTheme(theme)}>
                  {theme === 'system' ? '跟随系统' : theme === 'light' ? '浅色' : '深色'}
                </button>
              ))}
            </div>
          </div>
        </div>
      </Panel>
      <Panel title="运行环境" icon={<Globe2 size={20} />} tone="slate" className="settings-grid__wide">
        <div className="runtime-list">
          <Detail label="核心" value={coreVersion} />
          <Detail label="桥接" value={isNative ? '已连接原生核心' : '预览桥接'} />
        </div>
      </Panel>
      <Panel title="当前私有网络的核心选项" icon={<SlidersHorizontal size={20} />} tone="blue" className="settings-grid__wide flags-settings-panel">
        {activeProfile ? (
          <ProfileFlagsEditor
            key={`${activeProfile.id}:${activeProfile.updatedAt}`}
            profile={activeProfile}
            onSave={onSaveFlags}
          />
        ) : (
          <EmptyState icon={<Network size={26} />} label="请选择一个私有网络档案" actionLabel="打开私有网络" onAction={onOpenNetwork} />
        )}
      </Panel>
    </section>
  </>
)

const ProfileFlagsEditor = ({
  profile,
  onSave,
}: {
  profile: NetworkProfile
  onSave: (profileId: string, flags: EasyTierFlags) => Promise<NetworkProfile>
}) => {
  const [draft, setDraft] = useState<EasyTierFlags>(() => normalizeEasyTierFlags(profile.flags))
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const errorRef = useErrorFocus(error)
  const validationErrors = validateEasyTierFlags(draft)
  const hasValidationErrors = Object.keys(validationErrors).length > 0

  const update = (key: keyof EasyTierFlags, value: EasyTierFlags[keyof EasyTierFlags]) => {
    setDraft((current) => ({ ...current, [key]: value } as EasyTierFlags))
    setError('')
  }

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    if (hasValidationErrors) {
      setError(Object.values(validationErrors).find(Boolean) ?? '请先修正标红的核心选项。')
      return
    }

    setSaving(true)
    setError('')
    try {
      await onSave(profile.id, draft)
    } catch (saveError) {
      setError(getErrorMessage(saveError, '无法保存核心选项。'))
    } finally {
      setSaving(false)
    }
  }

  return (
    <form className="flags-editor" onSubmit={(event) => void submit(event)}>
      <div className="flags-editor__toolbar">
        <div>
          <span>正在编辑</span>
          <strong>{profile.name}</strong>
        </div>
        <button className="button button--primary" type="submit" disabled={saving || hasValidationErrors}>
          <Check size={17} /> {saving ? '保存中' : '保存核心选项'}
        </button>
      </div>

      <div className="flags-editor__sections">
        {easyTierFlagSections.map((section) => (
          <details className="flag-section" key={section.id} open>
            <summary>
              <div>
                <strong>{section.title}</strong>
                <span>{section.description}</span>
              </div>
              <span className="flag-section__count">{section.fields.length} 项</span>
            </summary>
            <div className="flag-section__body">
              {section.fields.map((field) => (
                <EasyTierFlagControl
                  key={field.key}
                  field={field}
                  value={draft[field.key]}
                  error={validationErrors[field.key]}
                  disabled={saving}
                  onChange={(value) => update(field.key, value)}
                />
              ))}
            </div>
          </details>
        ))}
      </div>

      {error && <p className="flags-editor__error" ref={errorRef} tabIndex={-1} role="alert">保存失败：{error}</p>}
      <footer className="flags-editor__footer">
        <button className="button button--primary" type="submit" disabled={saving || hasValidationErrors}>
          <Check size={17} /> {saving ? '保存中' : '保存核心选项'}
        </button>
      </footer>
    </form>
  )
}

const EasyTierFlagControl = ({
  field,
  value,
  error,
  disabled,
  onChange,
}: {
  field: EasyTierFlagField
  value: EasyTierFlags[keyof EasyTierFlags]
  error?: string
  disabled: boolean
  onChange: (value: EasyTierFlags[keyof EasyTierFlags]) => void
}) => {
  const inputClassName = error ? 'flag-field__input flag-field__input--invalid' : 'flag-field__input'
  const controlDisabled = disabled || field.readOnly === true

  const control = (() => {
    switch (field.kind) {
      case 'toggle':
        return (
          <Toggle
            checked={Boolean(value)}
            onChange={onChange}
            label={field.label}
            disabled={controlDisabled}
          />
        )
      case 'select':
        return (
          <select
            className={inputClassName}
            value={String(value)}
            aria-label={field.label}
            aria-invalid={Boolean(error)}
            disabled={controlDisabled}
            onChange={(event) => onChange(event.target.value)}
          >
            {field.options?.map((option) => (
              <option key={option.value} value={option.value}>{option.label}</option>
            ))}
          </select>
        )
      case 'number':
        return (
          <input
            className={inputClassName}
            type="number"
            value={Number(value)}
            min={field.min}
            max={field.max}
            step={field.step ?? 1}
            aria-label={field.label}
            aria-invalid={Boolean(error)}
            disabled={controlDisabled}
            onChange={(event) => onChange(Number(event.target.value))}
          />
        )
      case 'rate':
        return (
          <input
            className={inputClassName}
            type="text"
            inputMode="numeric"
            pattern="[0-9]*"
            value={rateLimitInputValue(String(value))}
            placeholder="无限制"
            aria-label={field.label}
            aria-invalid={Boolean(error)}
            disabled={controlDisabled}
            onChange={(event) => onChange(rateLimitFromInput(event.target.value))}
          />
        )
      default:
        return (
          <input
            className={inputClassName}
            type="text"
            value={String(value)}
            aria-label={field.label}
            aria-invalid={Boolean(error)}
            disabled={controlDisabled}
            onChange={(event) => onChange(event.target.value)}
          />
        )
    }
  })()

  return (
    <div className={error ? 'flag-field flag-field--invalid' : 'flag-field'}>
      <div className="flag-field__copy">
        <strong>{field.label}</strong>
        <span>{field.description}</span>
      </div>
      <div className="flag-field__control">
        {control}
        {error && <span className="flag-field__error">{error}</span>}
      </div>
    </div>
  )
}

const ProfileDialog = ({
  profile,
  onClose,
  onSave,
}: {
  profile: NetworkProfile | null
  onClose: () => void
  onSave: (draft: ProfileDraft) => Promise<void>
}) => {
  const [draft, setDraft] = useState<ProfileDraft>(profile ? profileToDraft(profile) : blankProfile)
  const [showSecret, setShowSecret] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const errorRef = useErrorFocus(error)
  const [bootstrapPeer, setBootstrapPeer] = useState('')
  const [selectedPeer, setSelectedPeer] = useState(profile?.peers[0] ?? '')
  const [peerError, setPeerError] = useState('')

  useEffect(() => {
    const handleKeyDown = (event: KeyboardEvent) => {
      if (event.key === 'Escape' && !saving) onClose()
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [onClose, saving])

  const update = <Key extends keyof ProfileDraft>(key: Key, value: ProfileDraft[Key]) => {
    setDraft((current) => ({ ...current, [key]: value }))
    setError('')
  }

  const addPeers = () => {
    const additions = splitBootstrapPeerInput(bootstrapPeer)
    if (!additions.length) {
      setPeerError('请填写至少一个 Bootstrap 节点地址。')
      return
    }

    const validationError = additions
      .map((peer) => validateBootstrapPeer(peer))
      .find((message): message is string => Boolean(message))
    if (validationError) {
      setPeerError(validationError)
      return
    }

    const existingPeers = normalizeBootstrapPeers(draft.peers)
    const peers = addBootstrapPeers(existingPeers, additions)
    if (peers.length > MAX_BOOTSTRAP_PEERS) {
      setPeerError(`最多可添加 ${MAX_BOOTSTRAP_PEERS} 个 Bootstrap 节点。`)
      return
    }

    const existingPeerSet = new Set(existingPeers)
    const addedPeers = peers.filter((peer) => !existingPeerSet.has(peer))
    if (!addedPeers.length) {
      setPeerError('填写的 Bootstrap 节点均已添加。')
      return
    }

    setDraft((current) => ({ ...current, peers }))
    setBootstrapPeer('')
    setSelectedPeer(addedPeers.at(-1) ?? '')
    setPeerError('')
    setError('')
  }

  const removeSelectedPeer = () => {
    if (!selectedPeer) return

    const peers = draft.peers.filter((peer) => peer !== selectedPeer)
    setDraft((current) => ({ ...current, peers }))
    setSelectedPeer(peers[0] ?? '')
    setPeerError('')
    setError('')
  }

  const submit = async (event: FormEvent<HTMLFormElement>) => {
    event.preventDefault()
    const normalizedPeers = normalizeBootstrapPeers(draft.peers)
    const invalidPeerMessage = normalizedPeers
      .map((peer) => validateBootstrapPeer(peer))
      .find((message): message is string => Boolean(message))
    const validationErrors = [
      !draft.name.trim() && '请填写档案名称。',
      !draft.networkName.trim() && '请填写网络名称。',
      !draft.networkSecret.trim() && '请填写网络密钥。',
      !normalizedPeers.length && '请至少添加一个 Bootstrap 节点。',
      normalizedPeers.length > MAX_BOOTSTRAP_PEERS && `Bootstrap 节点最多只能添加 ${MAX_BOOTSTRAP_PEERS} 个。`,
      invalidPeerMessage,
      !draft.virtualIp.trim() && '请填写固定虚拟 IPv4/CIDR。',
      draft.virtualIp.trim() && !isIpv4Cidr(draft.virtualIp.trim())
        && '固定虚拟 IPv4/CIDR 格式不正确，例如 10.147.18.24/24。',
    ].filter(Boolean)
    if (validationErrors.length) {
      setError(validationErrors.join(' '))
      return
    }
    setSaving(true)
    setError('')
    try {
      await onSave({
        ...draft,
        name: draft.name.trim(),
        deviceName: draft.deviceName.trim(),
        networkName: draft.networkName.trim(),
        networkSecret: draft.networkSecret.trim(),
        virtualIp: draft.virtualIp.trim(),
        peers: normalizedPeers,
    })
    } catch (saveError) {
      setError(getErrorMessage(saveError, '无法保存档案。'))
      setSaving(false)
    }
  }

  return (
    <div className="dialog-backdrop" role="presentation" onMouseDown={onClose}>
      <section className="profile-dialog" role="dialog" aria-modal="true" aria-labelledby="profile-dialog-title" onMouseDown={(event) => event.stopPropagation()}>
        <header className="profile-dialog__header">
          <div><span className="dialog-icon"><Network size={20} /></span><h2 id="profile-dialog-title">{profile ? '编辑档案' : '新建档案'}</h2></div>
          <IconButton title="关闭对话框" onClick={onClose} disabled={saving}><X size={18} /></IconButton>
        </header>
        <form onSubmit={(event) => void submit(event)}>
          <div className="form-grid">
            <label className="field"><span>档案名称</span><input value={draft.name} onChange={(event) => update('name', event.target.value)} autoFocus /></label>
            <label className="field"><span>设备名称</span><input value={draft.deviceName} onChange={(event) => update('deviceName', event.target.value)} placeholder="留空时使用 Windows 本机名称" /><small className="field__hint">留空时使用 Windows 本机名称。</small></label>
            <label className="field"><span>网络名称</span><input value={draft.networkName} onChange={(event) => update('networkName', event.target.value)} /></label>
            <label className="field field--wide"><span>{profile ? '网络密钥（再次输入后保存）' : '网络密钥'}</span><div className="secret-input"><input type={showSecret ? 'text' : 'password'} autoComplete="current-password" value={draft.networkSecret} onChange={(event) => update('networkSecret', event.target.value)} /><IconButton title={showSecret ? '隐藏网络密钥' : '显示网络密钥'} onClick={() => setShowSecret((visible) => !visible)}>{showSecret ? <EyeOff size={16} /> : <Eye size={16} />}</IconButton></div></label>
            <div className="field field--wide">
              <span>Bootstrap 节点</span>
              <div className="peer-entry">
                <textarea
                  value={bootstrapPeer}
                  onChange={(event) => {
                    setBootstrapPeer(event.target.value)
                    setPeerError('')
                  }}
                  onKeyDown={(event) => {
                    if (event.key === 'Enter' && (event.ctrlKey || event.metaKey)) {
                      event.preventDefault()
                      addPeers()
                    }
                  }}
                  placeholder={'tcp://203.0.113.10:11010\nwg://203.0.113.10:11012'}
                  aria-label="Bootstrap 节点地址"
                  aria-describedby="bootstrap-peer-hint"
                  rows={2}
                />
                <button className="button button--secondary" type="button" onClick={addPeers} disabled={saving}><Plus size={16} /> 添加</button>
              </div>
              <div className="peer-select-row">
                <select value={selectedPeer} onChange={(event) => setSelectedPeer(event.target.value)} aria-label="已添加的 Bootstrap 节点" disabled={!draft.peers.length || saving}>
                  <option value="" disabled>{draft.peers.length ? '请选择已添加的 Bootstrap 节点' : '尚未添加 Bootstrap 节点'}</option>
                  {draft.peers.map((peer) => <option key={peer} value={peer}>{peer}</option>)}
                </select>
                <IconButton title="移除选中的 Bootstrap 节点" onClick={removeSelectedPeer} disabled={!selectedPeer || saving}><Trash2 size={16} /></IconButton>
              </div>
              <small className="field__hint" id="bootstrap-peer-hint">支持 tcp://、udp://、wg://、ws:// 和 wss://。可用换行或逗号一次添加多个地址，也可为同一 Bootstrap 节点添加不同协议和端口，例如 TCP、UDP、WireGuard。Core 会并行尝试并维护可用传输，实际连接协议会显示在“节点”页；最多 {MAX_BOOTSTRAP_PEERS} 个。</small>
              {peerError && <span className="field__validation">{peerError}</span>}
            </div>
            <label className="field"><span>固定虚拟 IPv4 / CIDR</span><input value={draft.virtualIp} onChange={(event) => update('virtualIp', event.target.value)} placeholder="10.147.18.24/24" /></label>
          </div>
          {error && <p className="form-error" ref={errorRef} tabIndex={-1} role="alert">保存档案失败：{error}</p>}
          <footer className="profile-dialog__footer">
            <button className="button button--quiet" type="button" onClick={onClose} disabled={saving}>取消</button>
            <button className="button button--primary" type="submit" disabled={saving}><Check size={17} /> {saving ? '保存中' : '保存档案'}</button>
          </footer>
        </form>
      </section>
    </div>
  )
}
