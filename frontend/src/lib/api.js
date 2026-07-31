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

export async function apiEventStream(path, body, onEvent, options = {}) {
  const response = await fetch(`${API}${path}`, {
    ...options,
    method: 'POST',
    headers: { ...options.headers, 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  })
  if (!response.ok) {
    const payload = await response.json().catch(() => ({}))
    const error = new Error(payload.detail || response.statusText)
    error.status = response.status
    throw error
  }
  if (!response.body) throw new Error('浏览器不支持流式响应')

  const reader = response.body.getReader()
  const decoder = new TextDecoder()
  let buffer = ''
  const dispatch = (block) => {
    let event = 'message'
    const data = []
    for (const line of block.split(/\r?\n/)) {
      if (line.startsWith('event:')) event = line.slice(6).trim()
      if (line.startsWith('data:')) data.push(line.slice(5).trimStart())
    }
    if (!data.length) return
    const raw = data.join('\n')
    let payload = raw
    try {
      payload = JSON.parse(raw)
    } catch {
      // Keep non-JSON upstream diagnostics readable.
    }
    onEvent(event, payload)
  }

  while (true) {
    const { value, done } = await reader.read()
    buffer += decoder.decode(value || new Uint8Array(), { stream: !done })
    let boundary = buffer.search(/\r?\n\r?\n/)
    while (boundary >= 0) {
      const block = buffer.slice(0, boundary)
      const separator = buffer.slice(boundary).match(/^\r?\n\r?\n/)?.[0] || '\n\n'
      buffer = buffer.slice(boundary + separator.length)
      dispatch(block)
      boundary = buffer.search(/\r?\n\r?\n/)
    }
    if (done) break
  }
  if (buffer.trim()) dispatch(buffer)
}

export function websocketUrl(path) {
  if (API) return `${API.replace(/^http/, 'ws')}${path}`
  const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
  return `${protocol}//${window.location.host}${path}`
}
