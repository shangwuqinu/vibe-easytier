import type {
  BandwidthTestResult,
  DesktopSnapshot,
  EasyTierFlags,
  LogEntry,
  NetworkProfile,
  Preferences,
} from '../types'

/**
 * This is the only browser-to-native boundary used by the UI. The Tauri host
 * can expose compatible methods on window.__TAURI__.core without adding a
 * frontend dependency on Tauri's JavaScript packages. Command completion must
 * be followed by an authoritative getSnapshot result or a subscribeSnapshot
 * event; the UI never treats a command acknowledgement as connection proof.
 */
export interface EasyTierCoreBridge {
  getSnapshot?: () => Promise<Partial<DesktopSnapshot>>
  saveProfile?: (profile: NetworkProfile) => Promise<NetworkProfile>
  updateProfileFlags?: (profileId: string, flags: EasyTierFlags) => Promise<NetworkProfile>
  importProfileFromFile?: () => Promise<NetworkProfile | null>
  exportProfile?: (profileId: string) => Promise<string | null>
  runBandwidthTest?: (peerId: string) => Promise<BandwidthTestResult>
  deleteProfile?: (profileId: string) => Promise<void>
  selectProfile?: (profileId: string) => Promise<void>
  connect?: (profileId: string) => Promise<void>
  disconnect?: () => Promise<void>
  setAutoConnect?: (enabled: boolean) => Promise<void>
  setTheme?: (theme: Preferences['theme']) => Promise<void>
  clearLogs?: () => Promise<void>
  minimizeWindow?: () => Promise<void>
  toggleMaximizeWindow?: () => Promise<void>
  hideWindow?: () => Promise<void>
  subscribeSnapshot?: (listener: (snapshot: Partial<DesktopSnapshot>) => void) =>
    Promise<() => void> | (() => void)
}

type TauriInvoke = <T>(command: string, args?: Record<string, unknown>) => Promise<T>

declare global {
  interface Window {
    __TAURI__?: {
      core?: {
        invoke?: TauriInvoke
      }
    }
  }
}

const getInvoke = () => window.__TAURI__?.core?.invoke

const invoke = <T>(command: string, args?: Record<string, unknown>) => {
  const nativeInvoke = getInvoke()
  if (!nativeInvoke) {
    return Promise.reject(new Error('Vibe EasyTier 桌面桥接不可用。'))
  }
  return nativeInvoke<T>(command, args)
}

export const getCoreBridge = (): EasyTierCoreBridge | undefined => {
  if (!getInvoke()) return undefined

  return {
    getSnapshot: () => invoke<Partial<DesktopSnapshot>>('get_snapshot'),
    saveProfile: (profile) => invoke<NetworkProfile>('save_profile', { profile }),
    updateProfileFlags: (profileId, flags) => invoke<NetworkProfile>('update_profile_flags', { profileId, flags }),
    importProfileFromFile: () => invoke<NetworkProfile | null>('import_profile_from_file'),
    exportProfile: (profileId) => invoke<string | null>('export_profile_toml', { profileId }),
    runBandwidthTest: (peerId) => invoke<BandwidthTestResult>('run_peer_bandwidth_test', { peerId }),
    deleteProfile: (profileId) => invoke<void>('delete_profile', { profileId }),
    selectProfile: (profileId) => invoke<void>('select_active_profile', { profileId }),
    connect: (profileId) => invoke<void>('connect', { profileId }),
    disconnect: () => invoke<void>('disconnect'),
    setAutoConnect: (enabled) => invoke<void>('set_auto_connect', { enabled }),
    setTheme: (theme) => invoke<void>('set_theme', { theme }),
    clearLogs: () => invoke<void>('clear_logs'),
    minimizeWindow: () => invoke<void>('minimize_window'),
    toggleMaximizeWindow: () => invoke<void>('toggle_maximize_window'),
    hideWindow: () => invoke<void>('hide_window'),
    subscribeSnapshot: (listener) => {
      const timer = window.setInterval(() => {
        void invoke<Partial<DesktopSnapshot>>('get_snapshot').then(listener).catch(() => {
          // A later poll will recover after a service restart.
        })
      }, 5_000)
      return () => window.clearInterval(timer)
    },
  }
}

export const hasNativeBridge = () => Boolean(getInvoke())

export const makeLog = (
  level: LogEntry['level'],
  source: LogEntry['source'],
  message: string,
): LogEntry => ({
  id: crypto.randomUUID(),
  at: new Date().toISOString(),
  level,
  source,
  message,
})
