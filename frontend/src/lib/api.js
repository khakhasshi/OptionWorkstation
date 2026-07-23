const API = import.meta.env.VITE_API_BASE || ''

export async function api(path, options) {
  const request = options instanceof AbortSignal ? { signal: options } : (options || {})
  const response = await fetch(`${API}${path}`, request)
  const payload = await response.json().catch(() => ({}))
  if (!response.ok) {
    const error = new Error(payload.detail || response.statusText)
    error.status = response.status
    error.retryAfterMs = payload.retry_after_ms
    throw error
  }
  return payload
}

export function apiJson(path, method, body, options = {}) {
  return api(path, {
    ...options,
    method,
    headers: { ...options.headers, 'Content-Type': 'application/json' },
    body: body == null ? undefined : JSON.stringify(body),
  })
}

export function websocketUrl(path) {
  if (API) return `${API.replace(/^http/, 'ws')}${path}`
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${protocol}//${window.location.host}${path}`
}
