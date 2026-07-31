const errorKeys = ['message', 'error', 'reason', 'detail', 'details', 'cause'] as const

const textFrom = (value: unknown): string | undefined => {
  if (typeof value !== 'string') return undefined
  const text = value.trim()
  return text || undefined
}

const safeProperty = (value: object, key: string): unknown => {
  try {
    return Reflect.get(value, key)
  } catch {
    return undefined
  }
}

const messageFrom = (value: unknown, seen: Set<object>, depth: number): string | undefined => {
  const direct = textFrom(value)
  if (direct) return direct
  if (!value || typeof value !== 'object' || depth >= 4 || seen.has(value)) return undefined

  seen.add(value)

  let hasErrorProperty = false
  for (const key of errorKeys) {
    const candidate = safeProperty(value, key)
    hasErrorProperty ||= candidate !== undefined
    const nested = messageFrom(candidate, seen, depth + 1)
    if (nested) return nested
  }

  if (hasErrorProperty) return undefined

  try {
    const serialized = JSON.stringify(value)
    if (serialized && serialized !== '{}' && serialized !== '[]' && serialized !== 'null') {
      return serialized
    }
  } catch {
    // A rejected value can be a cyclic object or a proxy. Fall back safely.
  }

  return undefined
}

/**
 * Tauri command rejections can arrive as Error instances, strings, or JSON
 * values. Normalize them without assuming a particular serialization shape.
 */
export const getErrorMessage = (error: unknown, fallback: string): string =>
  messageFrom(error, new Set<object>(), 0) ?? fallback
