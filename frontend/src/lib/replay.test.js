import assert from 'node:assert/strict'
import test from 'node:test'
import { createReplayController } from './replay.js'

function fakeClock() {
  let now = 0
  let nextId = 0
  const timers = new Map()
  return {
    setTimeout(callback, delay) {
      const id = ++nextId
      timers.set(id, { callback, at: now + delay })
      return id
    },
    clearTimeout(id) { timers.delete(id) },
    advance(ms) {
      const target = now + ms
      while (true) {
        const next = [...timers].sort((a, b) => a[1].at - b[1].at)[0]
        if (!next || next[1].at > target) break
        now = next[1].at
        timers.delete(next[0])
        next[1].callback()
      }
      now = target
    },
  }
}

const flush = async () => {
  // Drain the controller's promise adoption and fulfillment/error callbacks.
  for (let index = 0; index < 8; index += 1) await Promise.resolve()
}

function harness() {
  const clock = fakeClock()
  const pending = []
  const commits = []
  const errors = []
  const requestedFrames = []
  let stops = 0
  let request = { contextKey: 'SPY:2026-03-13:2026-03-20:micro:classic', session: {}, frame: 0, lastFrame: 8 }
  let options = { playing: false, speed: 30 }
  const controller = createReplayController({
    clock,
    load: (next, signal) => new Promise((resolve, reject) => {
      pending.push({ request: next, signal, resolve, reject })
    }),
    onCommit: (next, data) => commits.push({ ...next, data }),
    onFrame: (frame) => {
      requestedFrames.push(frame)
      request = { ...request, frame }
      controller.update(request, options)
    },
    onStop: () => {
      stops += 1
      options = { ...options, playing: false }
      controller.update(request, options)
    },
    onError: (reason) => errors.push(reason.message),
  })
  return {
    clock, pending, commits, errors, requestedFrames, controller,
    get stops() { return stops },
    update(patch = {}, settings = {}) {
      request = patch === null ? null : { ...request, ...patch }
      options = { ...options, ...settings }
      controller.update(request, options)
    },
    async resolve(index, data = { snapshot_id: `snapshot-${index}` }) {
      pending[index].resolve(data)
      await flush()
    },
  }
}

test('30x playback commits every frame and waits for slow snapshots', async () => {
  const h = harness()
  h.update({}, { playing: true })
  await flush()
  h.clock.advance(1000)
  assert.equal(h.pending.length, 1)
  assert.equal(h.pending[0].signal.aborted, false)
  assert.equal(h.commits.length, 0)

  for (let frame = 0; frame < 3; frame += 1) {
    const data = { chain: { frame }, surface: { frame }, volatility: { frame } }
    await h.resolve(frame, data)
    assert.equal(h.commits.at(-1).frame, frame)
    assert.equal(h.commits.at(-1).data, data)
    h.clock.advance(34)
    await flush()
    assert.equal(h.pending.length, frame + 2)
    h.clock.advance(500)
    assert.equal(h.pending.length, frame + 2)
    assert.equal(h.pending.at(-1).signal.aborted, false)
  }
  assert.deepEqual(h.commits.map((item) => item.frame), [0, 1, 2])
  assert.deepEqual(h.requestedFrames, [1, 2, 3])
  h.controller.dispose()
})

test('manual seeks stop playback and only the latest result can commit', async () => {
  const h = harness()
  h.update({}, { playing: true })
  await flush()
  h.update({ frame: 5 }, { playing: false })
  await flush()
  h.update({ frame: 2 }, { playing: false })
  await flush()
  assert.equal(h.pending[0].signal.aborted, true)
  assert.equal(h.pending[1].signal.aborted, true)
  await h.resolve(2)
  // An API can still resolve after cancellation; it must not roll back the UI.
  await h.resolve(1)
  await h.resolve(0)
  h.clock.advance(1000)
  assert.deepEqual(h.commits.map((item) => item.frame), [2])
  assert.deepEqual(h.requestedFrames, [])
})

test('context changes invalidate both pending requests and scheduled advances', async () => {
  const h = harness()
  h.update({}, { playing: true })
  await flush()
  await h.resolve(0)
  h.update({ contextKey: 'SPY:2026-03-16:2026-03-20:mid:short_all' })
  await flush()
  h.clock.advance(1000)
  assert.equal(h.pending.length, 2)
  assert.deepEqual(h.requestedFrames, [])
  h.update({ contextKey: 'QQQ:2026-03-16:2026-03-20:mid:short_all' })
  await flush()
  assert.equal(h.pending[1].signal.aborted, true)
  await h.resolve(1)
  assert.equal(h.commits.length, 1)
  await h.resolve(2)
  assert.equal(h.commits.at(-1).contextKey, 'QQQ:2026-03-16:2026-03-20:mid:short_all')
  h.controller.dispose()
})

test('pausing and changing speed keep a valid in-flight snapshot', async () => {
  const h = harness()
  h.update({}, { playing: true })
  await flush()
  h.update({}, { speed: 5, playing: false })
  assert.equal(h.pending[0].signal.aborted, false)
  await h.resolve(0)
  h.clock.advance(1000)
  assert.equal(h.pending.length, 1)
  h.update({}, { playing: true })
  h.clock.advance(199)
  assert.deepEqual(h.requestedFrames, [])
  h.clock.advance(1)
  await flush()
  assert.deepEqual(h.requestedFrames, [1])
  assert.equal(h.pending.length, 2)
  h.controller.dispose()
})

test('a failed frame stops playback without advancing and can be retried', async () => {
  const h = harness()
  h.update({}, { playing: true })
  await flush()
  h.pending[0].reject(new Error('snapshot unavailable'))
  await flush()
  h.clock.advance(1000)
  assert.deepEqual(h.errors, ['snapshot unavailable'])
  assert.equal(h.stops, 1)
  assert.deepEqual(h.requestedFrames, [])
  h.update({}, { playing: true })
  await flush()
  assert.equal(h.pending.length, 2)
  await h.resolve(1)
  assert.deepEqual(h.commits.map((item) => item.frame), [0])
  h.controller.dispose()
})

test('the final committed frame stops playback without an out-of-range request', async () => {
  const h = harness()
  h.update({ frame: 8 }, { playing: true })
  await flush()
  await h.resolve(0)
  h.clock.advance(1000)
  assert.equal(h.stops, 1)
  assert.equal(h.pending.length, 1)
  assert.deepEqual(h.requestedFrames, [])
  assert.equal(h.commits[0].frame, 8)
})

test('a missing session and unmount abort requests and ignore late failures or results', async () => {
  const h = harness()
  h.update({}, { playing: true })
  await flush()
  h.update(null)
  assert.equal(h.pending[0].signal.aborted, true)
  h.pending[0].reject(new Error('old session failure'))
  await flush()
  assert.deepEqual(h.errors, [])
  h.update({ contextKey: 'new', session: {}, frame: 0, lastFrame: 5 }, { playing: true })
  await flush()
  h.controller.dispose()
  assert.equal(h.pending[1].signal.aborted, true)
  await h.resolve(1)
  h.clock.advance(1000)
  assert.deepEqual(h.commits, [])
  assert.deepEqual(h.requestedFrames, [])
})
