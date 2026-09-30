// Playback waits for a complete snapshot before requesting the next frame.
// Seeks replace the request; changing playback speed never cancels its data.
export function createReplayController({ load, onCommit, onFrame, onStop, onError, clock = globalThis }) {
  let current = null
  let controller = null
  let timer = null
  let generation = 0
  let playing = false
  let speed = 1
  let phase = 'idle'
  let disposed = false

  const clearTimer = () => {
    if (timer != null) clock.clearTimeout(timer)
    timer = null
  }
  const cancel = () => {
    clearTimer()
    generation += 1
    controller?.abort()
    controller = null
  }
  const scheduleNext = () => {
    clearTimer()
    if (!playing || phase !== 'ready' || !current) return
    if (current.frame >= current.lastFrame) {
      playing = false
      onStop()
      return
    }
    timer = clock.setTimeout(() => {
      timer = null
      phase = 'advancing'
      onFrame(current.frame + 1)
    }, Math.max(32, 1000 / speed))
  }
  const requestSnapshot = () => {
    cancel()
    const request = current
    const requestGeneration = generation
    const abortController = new AbortController()
    controller = abortController
    phase = 'loading'
    const isCurrent = () => !disposed && generation === requestGeneration && !abortController.signal.aborted
    Promise.resolve().then(() => {
      if (isCurrent()) return load(request, abortController.signal)
    }).then((data) => {
      if (!isCurrent()) return
      controller = null
      phase = 'ready'
      onCommit(request, data)
      if (isCurrent()) scheduleNext()
    }).catch((reason) => {
      if (!isCurrent()) return
      controller = null
      phase = 'failed'
      playing = false
      onError(reason)
      onStop()
    })
  }

  return {
    update(request, options) {
      if (disposed) return
      const playbackChanged = playing !== options.playing || speed !== options.speed
      const resume = !playing && options.playing
      playing = options.playing
      speed = options.speed
      if (!request) {
        cancel()
        current = null
        phase = 'idle'
        if (playing) {
          playing = false
          onStop()
        }
        return
      }
      const changed = !current || current.contextKey !== request.contextKey
        || current.session !== request.session || current.frame !== request.frame
      current = request
      if (changed || (phase === 'failed' && resume)) requestSnapshot()
      else if (playbackChanged) scheduleNext()
    },
    dispose() {
      disposed = true
      cancel()
      current = null
    },
  }
}
