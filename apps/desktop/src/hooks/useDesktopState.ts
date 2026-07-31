import { useCallback, useEffect, useRef, useState } from 'react'

import { canConnect } from '../lib/connection'
import { getCoreBridge, hasNativeBridge, makeLog } from '../lib/bridge'
import type {
  DesktopSnapshot,
  EasyTierFlags,
  NetworkProfile,
  Preferences,
  ProfileDraft,
} from '../types'

const PREFERENCES_STORAGE_KEY = 'easytier.desktop.preferences.v1'
const LEGACY_SNAPSHOT_STORAGE_KEY = 'easytier.desktop.snapshot.v1'

const now = () => new Date().toISOString()

const createSeedSnapshot = (): DesktopSnapshot => {
  return {
    profiles: [],
    peers: [],
    runtime: {
      phase: 'disconnected',
      activeProfileId: null,
      startedAt: null,
      retryAt: null,
      error: null,
      peerCount: 0,
      peerCountAvailable: false,
      lastSuccessAt: null,
      routes: 0,
      sent: 0,
      received: 0,
      daemonVersion: '正在等待 EasyTier Core',
    },
    preferences: {
      autoConnect: false,
      serviceAtBoot: false,
      serviceHealth: 'unavailable',
      theme: 'system',
    },
    logs: [
      makeLog('info', 'Desktop', '预览模式处于离线状态，等待 EasyTier Core 可用。'),
    ],
  }
}

const loadSnapshot = (): DesktopSnapshot => {
  const seed = createSeedSnapshot()
  try {
    // Retire the old preview storage shape because it could contain a secret.
    window.localStorage.removeItem(LEGACY_SNAPSHOT_STORAGE_KEY)
    const raw = window.localStorage.getItem(PREFERENCES_STORAGE_KEY)
    if (!raw) return seed
    const preferences = JSON.parse(raw) as Partial<Preferences>
    return {
      ...seed,
      preferences: { ...seed.preferences, ...preferences },
    }
  } catch {
    return seed
  }
}

const mergeSnapshot = (
  current: DesktopSnapshot,
  patch: Partial<DesktopSnapshot>,
): DesktopSnapshot => ({
  ...current,
  ...patch,
  profiles: patch.profiles ?? current.profiles,
  peers: patch.peers ?? current.peers,
  logs: patch.logs ?? current.logs,
  runtime: { ...current.runtime, ...patch.runtime },
  preferences: { ...current.preferences, ...patch.preferences },
})

const resolveTheme = (theme: Preferences['theme']) => {
  if (theme !== 'system') return theme
  return window.matchMedia('(prefers-color-scheme: dark)').matches ? 'dark' : 'light'
}

export const useDesktopState = () => {
  const [snapshot, setSnapshot] = useState<DesktopSnapshot>(loadSnapshot)
  const [isHydrated, setIsHydrated] = useState(false)
  const autoConnectAttempted = useRef(false)

  const mutate = useCallback(
    (recipe: (current: DesktopSnapshot) => DesktopSnapshot) => {
      setSnapshot((current) => recipe(current))
    },
    [],
  )

  const appendLog = useCallback(
    (entry: ReturnType<typeof makeLog>) => {
      mutate((current) => ({
        ...current,
        logs: [entry, ...current.logs].slice(0, 240),
      }))
    },
    [mutate],
  )

  useEffect(() => {
    try {
      // Network material remains in memory or the native secure store only.
      window.localStorage.setItem(
        PREFERENCES_STORAGE_KEY,
        JSON.stringify(snapshot.preferences),
      )
    } catch {
      // The native bridge remains usable when web storage is unavailable.
    }
  }, [snapshot])

  useEffect(() => {
    const theme = resolveTheme(snapshot.preferences.theme)
    document.documentElement.dataset.theme = theme
    document.documentElement.style.colorScheme = theme
  }, [snapshot.preferences.theme])

  useEffect(() => {
    let mounted = true
    let unsubscribe: (() => void) | undefined
    const bridge = getCoreBridge()

    const initialize = async () => {
      try {
        const remoteSnapshot = await bridge?.getSnapshot?.()
        if (mounted && remoteSnapshot) {
          setSnapshot((current) => mergeSnapshot(current, remoteSnapshot))
        }

        const release = await bridge?.subscribeSnapshot?.((patch) => {
          if (mounted) {
            setSnapshot((current) => mergeSnapshot(current, patch))
          }
        })
        if (typeof release === 'function') unsubscribe = release
      } catch (error) {
        if (mounted) {
          appendLog(
            makeLog(
              'warning',
              'Desktop',
              `无法同步核心状态：${error instanceof Error ? error.message : '未知错误'}`,
            ),
          )
        }
      } finally {
        if (mounted) setIsHydrated(true)
      }
    }

    void initialize()

    return () => {
      mounted = false
      unsubscribe?.()
    }
  }, [appendLog])

  const connect = useCallback(
    async (reason: 'manual' | 'automatic' = 'manual') => {
      const profile = snapshot.profiles.find(
        (item) => item.id === snapshot.runtime.activeProfileId,
      )
      if (!profile || !canConnect(snapshot.runtime.phase)) return

      mutate((current) => ({
        ...current,
        runtime: {
          ...current.runtime,
          phase: 'connecting',
          error: null,
          retryAt: null,
        },
      }))
      appendLog(
        makeLog(
          'info',
          reason === 'automatic' ? 'Reconnect' : 'Desktop',
          `正在连接到 ${profile.name}。`,
        ),
      )

      try {
        const bridge = getCoreBridge()
        if (!bridge?.connect) {
          throw new Error('EasyTier Core 不可用。')
        }
        await bridge.connect(profile.id)

        appendLog(
          makeLog(
            'info',
            'EasyTier Core',
            `已请求连接 ${profile.name}，正在等待核心状态。`,
          ),
        )
      } catch (error) {
        const message = error instanceof Error ? error.message : '连接请求失败。'
        mutate((current) => ({
          ...current,
          runtime: {
            ...current.runtime,
            phase: 'failed',
            error: message,
            retryAt: current.preferences.autoConnect
              ? new Date(Date.now() + 30_000).toISOString()
              : null,
          },
        }))
        appendLog(makeLog('error', 'EasyTier Core', message))
      }
    },
    [appendLog, mutate, snapshot.profiles, snapshot.runtime.activeProfileId, snapshot.runtime.phase],
  )

  const disconnect = useCallback(async () => {
    if (snapshot.runtime.phase === 'disconnected') return

    mutate((current) => ({
      ...current,
      runtime: { ...current.runtime, phase: 'disconnecting' },
    }))
    appendLog(makeLog('info', 'Desktop', '正在断开私有网络。'))

    try {
      const bridge = getCoreBridge()
      if (!bridge?.disconnect) {
        mutate((current) => ({
          ...current,
          runtime: {
            ...current.runtime,
            phase: 'disconnected',
            startedAt: null,
            routes: 0,
            retryAt: null,
          },
        }))
        appendLog(makeLog('info', 'Desktop', '预览模式离线，未向核心发送断开请求。'))
        return
      }
      await bridge.disconnect()
      appendLog(makeLog('info', 'EasyTier Core', '已请求断开连接，正在等待核心状态。'))
    } catch (error) {
      const message = error instanceof Error ? error.message : '断开连接请求失败。'
      mutate((current) => ({
        ...current,
        runtime: { ...current.runtime, phase: 'failed', error: message },
      }))
      appendLog(makeLog('error', 'EasyTier Core', message))
    }
  }, [appendLog, mutate, snapshot.runtime.phase])

  const saveProfile = useCallback(
    async (draft: ProfileDraft, id?: string) => {
      const profile: NetworkProfile = {
        ...draft,
        id: id ?? crypto.randomUUID(),
        updatedAt: now(),
      }

      const bridge = getCoreBridge()
      if (!bridge?.saveProfile) {
        const message = '浏览器预览不会保存私有网络档案，请在 Vibe EasyTier 桌面应用中创建或导入。'
        appendLog(makeLog('warning', 'Desktop', message))
        throw new Error(message)
      }
      const saved = await bridge.saveProfile(profile)
      mutate((current) => {
        const exists = current.profiles.some((item) => item.id === saved.id)
        const profiles = exists
          ? current.profiles.map((item) => (item.id === saved.id ? saved : item))
          : [saved, ...current.profiles]
        return {
          ...current,
          profiles,
          runtime: {
            ...current.runtime,
            activeProfileId: current.runtime.activeProfileId ?? saved.id,
          },
        }
      })
      appendLog(
        makeLog(
          'success',
          'Desktop',
          `已${id ? '更新' : '创建'}档案 ${saved.name}。`,
        ),
      )
      return saved
    },
    [appendLog, mutate],
  )

  const updateProfileFlags = useCallback(
    async (profileId: string, flags: EasyTierFlags) => {
      const bridge = getCoreBridge()
      if (!bridge?.updateProfileFlags) {
        const message = '当前安装的服务不支持更新 Core 选项。'
        appendLog(makeLog('warning', 'Desktop', message))
        throw new Error(message)
      }

      const saved = await bridge.updateProfileFlags(profileId, flags)
      mutate((current) => ({
        ...current,
        profiles: current.profiles.map((profile) => (
          profile.id === saved.id ? saved : profile
        )),
      }))
      appendLog(makeLog('success', 'Desktop', `已更新档案 ${saved.name} 的 Core 选项。`))
      return saved
    },
    [appendLog, mutate],
  )

  const importProfile = useCallback(
    async (toml: string) => {
      const bridge = getCoreBridge()
      if (!bridge?.importProfile) {
        throw new Error('仅原生 Vibe EasyTier 桌面应用支持导入档案。')
      }
      const saved = await bridge.importProfile(toml)
      mutate((current) => {
        const exists = current.profiles.some((item) => item.id === saved.id)
        const profiles = exists
          ? current.profiles.map((item) => (item.id === saved.id ? saved : item))
          : [saved, ...current.profiles]
        return {
          ...current,
          profiles,
          runtime: {
            ...current.runtime,
            activeProfileId: current.runtime.activeProfileId ?? saved.id,
          },
        }
      })
      appendLog(makeLog('success', 'Desktop', `已导入档案 ${saved.name}。`))
      return saved
    },
    [appendLog, mutate],
  )

  const deleteProfile = useCallback(
    async (profileId: string) => {
      await getCoreBridge()?.deleteProfile?.(profileId)
      mutate((current) => {
        const profiles = current.profiles.filter((item) => item.id !== profileId)
        const deletingActive = current.runtime.activeProfileId === profileId
        return {
          ...current,
          profiles,
          runtime: {
            ...current.runtime,
            activeProfileId: deletingActive ? (profiles[0]?.id ?? null) : current.runtime.activeProfileId,
            phase: deletingActive ? 'disconnected' : current.runtime.phase,
            startedAt: deletingActive ? null : current.runtime.startedAt,
            routes: deletingActive ? 0 : current.runtime.routes,
          },
        }
      })
      appendLog(makeLog('warning', 'Desktop', '已删除一个私有网络档案。'))
    },
    [appendLog, mutate],
  )

  const selectProfile = useCallback(
    async (profileId: string) => {
      try {
        const bridge = getCoreBridge()
        if (bridge && !bridge.selectProfile) {
          throw new Error('当前安装的服务不支持选择档案。')
        }
        await bridge?.selectProfile?.(profileId)
      } catch (error) {
        appendLog(
          makeLog(
            'error',
            'Desktop',
            `无法选择私有网络：${error instanceof Error ? error.message : '未知错误'}`,
          ),
        )
        return
      }
      mutate((current) => ({
        ...current,
        runtime: {
          ...current.runtime,
          activeProfileId: profileId,
          phase:
            current.runtime.phase === 'connected' ? 'disconnected' : current.runtime.phase,
          startedAt: current.runtime.phase === 'connected' ? null : current.runtime.startedAt,
          routes: current.runtime.phase === 'connected' ? 0 : current.runtime.routes,
        },
      }))
      appendLog(makeLog('info', 'Desktop', '已选择私有网络档案。'))
    },
    [appendLog, mutate],
  )

  const setAutoConnect = useCallback(
    async (enabled: boolean) => {
      mutate((current) => ({
        ...current,
        preferences: { ...current.preferences, autoConnect: enabled },
      }))
      try {
        const bridge = getCoreBridge()
        if (!bridge?.setAutoConnect) {
          appendLog(
            makeLog(
              'warning',
              'Desktop',
              '已在预览模式修改自动连接，EasyTier Core 不可用。',
            ),
          )
          return
        }
        await bridge.setAutoConnect(enabled)
        appendLog(
          makeLog(
            'success',
            'Desktop',
            `自动连接已${enabled ? '开启' : '关闭'}。`,
          ),
        )
      } catch (error) {
        mutate((current) => ({
          ...current,
          preferences: { ...current.preferences, autoConnect: !enabled },
        }))
        appendLog(
          makeLog(
            'error',
            'Desktop',
            `无法更新自动连接：${error instanceof Error ? error.message : '未知错误'}`,
          ),
        )
      }
    },
    [appendLog, mutate],
  )

  const setTheme = useCallback(
    async (theme: Preferences['theme']) => {
      mutate((current) => ({
        ...current,
        preferences: { ...current.preferences, theme },
      }))
      try {
        await getCoreBridge()?.setTheme?.(theme)
      } catch (error) {
        appendLog(
          makeLog(
            'warning',
            'Desktop',
            `无法同步主题：${error instanceof Error ? error.message : '未知错误'}`,
          ),
        )
      }
    },
    [appendLog, mutate],
  )

  const clearLogs = useCallback(async () => {
    await getCoreBridge()?.clearLogs?.()
    mutate((current) => ({ ...current, logs: [] }))
  }, [mutate])

  useEffect(() => {
    if (hasNativeBridge() || !isHydrated || !snapshot.preferences.autoConnect || autoConnectAttempted.current) return
    if (!snapshot.runtime.activeProfileId || snapshot.runtime.phase !== 'disconnected') return

    autoConnectAttempted.current = true
    const timer = window.setTimeout(() => {
      void connect('automatic')
    }, 480)
    return () => window.clearTimeout(timer)
  }, [connect, isHydrated, snapshot.preferences.autoConnect, snapshot.runtime.activeProfileId, snapshot.runtime.phase])

  return {
    snapshot,
    isHydrated,
    isNative: hasNativeBridge(),
    connect,
    disconnect,
    saveProfile,
    updateProfileFlags,
    importProfile,
    deleteProfile,
    selectProfile,
    setAutoConnect,
    setTheme,
    clearLogs,
  }
}
