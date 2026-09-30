// One pending frame per chart. Hidden charts retain only the latest input and
// keep their existing instance so user-controlled views survive scrolling.
export function createChartScheduler({ create, render, resize, dispose, requestFrame, cancelFrame }) {
  let instance = null
  let frame = null
  let visible = false
  let disposed = false
  let width = 0
  let height = 0
  let appliedWidth = 0
  let appliedHeight = 0
  let latest = null
  let revision = 0
  let appliedRevision = 0
  let resetPending = false

  const cancel = () => {
    if (frame != null) cancelFrame(frame)
    frame = null
  }
  const schedule = () => {
    if (disposed || frame != null || !visible || width <= 0 || height <= 0) return
    if (!instance && !latest?.option) return
    if (instance && revision === appliedRevision && width === appliedWidth && height === appliedHeight) return
    frame = requestFrame(flush)
  }
  const flush = () => {
    frame = null
    if (disposed || !visible || width <= 0 || height <= 0) return
    const targetWidth = width
    const targetHeight = height
    if (!instance) {
      if (!latest?.option) return
      instance = create(targetWidth, targetHeight)
      appliedWidth = targetWidth
      appliedHeight = targetHeight
    } else if (targetWidth !== appliedWidth || targetHeight !== appliedHeight) {
      resize(instance, targetWidth, targetHeight)
      appliedWidth = targetWidth
      appliedHeight = targetHeight
    }
    if (revision !== appliedRevision) {
      const targetRevision = revision
      const input = latest
      const reset = resetPending
      resetPending = false
      render(instance, input, reset)
      appliedRevision = targetRevision
    }
    schedule()
  }

  return {
    update(input) {
      if (disposed) return
      if (latest && latest.viewKey !== input.viewKey) resetPending = true
      latest = input
      revision += 1
      schedule()
    },
    setVisible(value) {
      if (disposed) return
      visible = value
      if (!visible) cancel()
      else schedule()
    },
    setSize(nextWidth, nextHeight) {
      if (disposed) return
      const roundedWidth = Math.max(0, Math.round(nextWidth))
      const roundedHeight = Math.max(0, Math.round(nextHeight))
      if (roundedWidth === width && roundedHeight === height) return
      width = roundedWidth
      height = roundedHeight
      if (width === 0 || height === 0) cancel()
      else schedule()
    },
    dispose() {
      if (disposed) return
      disposed = true
      cancel()
      if (instance) dispose(instance)
      instance = null
      latest = null
    },
  }
}

// ECharts listeners stay bound while React replaces callback functions.
export function createChartEventBindings(instance) {
  let handlers = {}
  let disposed = false
  const listeners = new Map()
  return {
    update(next) {
      if (disposed) return
      handlers = next
      for (const [name, listener] of listeners) {
        if (typeof handlers[name] !== 'function') {
          instance.off(name, listener)
          listeners.delete(name)
        }
      }
      for (const [name, handler] of Object.entries(handlers)) {
        if (typeof handler !== 'function' || listeners.has(name)) continue
        const listener = (...args) => handlers[name]?.(...args)
        listeners.set(name, listener)
        instance.on(name, listener)
      }
    },
    dispose() {
      if (disposed) return
      disposed = true
      for (const [name, listener] of listeners) instance.off(name, listener)
      listeners.clear()
      handlers = {}
    },
  }
}
