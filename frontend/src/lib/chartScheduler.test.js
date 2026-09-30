import assert from 'node:assert/strict'
import test from 'node:test'
import { createChartEventBindings, createChartScheduler } from './chartScheduler.js'

function harness() {
  const frames = new Map()
  const callbacks = []
  const calls = []
  let frameId = 0
  const instance = { id: 'chart' }
  const scheduler = createChartScheduler({
    requestFrame(callback) {
      frames.set(++frameId, callback)
      callbacks.push(callback)
      return frameId
    },
    cancelFrame(frame) { frames.delete(frame) },
    create(width, height) { calls.push(['create', width, height]); return instance },
    render(chart, input, reset) { assert.equal(chart, instance); calls.push(['render', input, reset]) },
    resize(chart, width, height) { assert.equal(chart, instance); calls.push(['resize', width, height]) },
    dispose(chart) { assert.equal(chart, instance); calls.push(['dispose']) },
  })
  return {
    scheduler, frames, callbacks, calls,
    update(version, viewKey = 'SPY:day') {
      scheduler.update({ option: version == null ? null : { version }, viewKey })
    },
    flush() {
      const pending = [...frames.values()]
      frames.clear()
      for (const callback of pending) callback()
    },
  }
}

test('offscreen charts do no work and first visibility draws only the latest option', () => {
  const h = harness()
  h.scheduler.setSize(640, 320)
  h.update(1)
  h.update(2)
  h.flush()
  assert.equal(h.frames.size, 0)
  assert.deepEqual(h.calls, [])
  h.scheduler.setVisible(true)
  h.update(3)
  assert.equal(h.frames.size, 1)
  h.flush()
  assert.deepEqual(h.calls, [
    ['create', 640, 320],
    ['render', { option: { version: 3 }, viewKey: 'SPY:day' }, false],
  ])
})

test('hiding cancels pending updates while keeping the instance and view for re-entry', () => {
  const h = harness()
  h.scheduler.setSize(640, 320)
  h.scheduler.setVisible(true)
  h.update(1)
  h.flush()
  h.update(2)
  h.scheduler.setVisible(false)
  h.update(3)
  h.scheduler.setSize(700, 340)
  h.flush()
  assert.equal(h.calls.length, 2)
  h.scheduler.setVisible(true)
  h.update(4)
  h.flush()
  assert.deepEqual(h.calls.slice(2), [
    ['resize', 700, 340],
    ['render', { option: { version: 4 }, viewKey: 'SPY:day' }, false],
  ])
  assert.equal(h.calls.filter(([kind]) => kind === 'create').length, 1)
})

test('resizes coalesce, unchanged dimensions do not resize, and resize does not replay options', () => {
  const h = harness()
  h.scheduler.setVisible(true)
  h.scheduler.setSize(640, 320)
  h.update(1)
  h.flush()
  h.scheduler.setSize(640.1, 320.2)
  h.flush()
  assert.equal(h.calls.length, 2)
  h.scheduler.setSize(700, 340)
  h.scheduler.setSize(720, 350)
  h.scheduler.setSize(740, 360)
  assert.equal(h.frames.size, 1)
  h.flush()
  assert.deepEqual(h.calls.slice(2), [['resize', 740, 360]])
})

test('zero-sized charts wait for layout and empty data does not initialize a renderer', () => {
  const h = harness()
  h.scheduler.setVisible(true)
  h.update(1)
  h.flush()
  assert.deepEqual(h.calls, [])
  h.scheduler.setSize(640, 0)
  h.flush()
  assert.deepEqual(h.calls, [])
  h.update(null)
  h.scheduler.setSize(640, 320)
  h.flush()
  assert.deepEqual(h.calls, [])
  h.update(2)
  h.flush()
  h.update(null)
  h.flush()
  assert.deepEqual(h.calls.at(-1), ['render', { option: null, viewKey: 'SPY:day' }, false])
})

test('view changes reset on the next visible render, including hidden round trips', () => {
  const h = harness()
  h.scheduler.setSize(640, 320)
  h.scheduler.setVisible(true)
  h.update(1, 'A')
  h.flush()
  h.scheduler.setVisible(false)
  h.update(2, 'B')
  h.update(3, 'A')
  h.scheduler.setVisible(true)
  h.flush()
  assert.deepEqual(h.calls.at(-1), ['render', { option: { version: 3 }, viewKey: 'A' }, true])
  h.update(4, 'A')
  h.flush()
  assert.deepEqual(h.calls.at(-1), ['render', { option: { version: 4 }, viewKey: 'A' }, false])
})

test('dispose cancels pending work and even late observer/frame callbacks cannot revive a chart', () => {
  const h = harness()
  h.scheduler.setSize(640, 320)
  h.scheduler.setVisible(true)
  h.update(1)
  h.flush()
  h.update(2)
  const lateCallback = h.callbacks.at(-1)
  h.scheduler.dispose()
  h.scheduler.dispose()
  h.scheduler.setVisible(true)
  h.scheduler.setSize(900, 400)
  h.update(3)
  lateCallback()
  h.flush()
  assert.deepEqual(h.calls.map(([kind]) => kind), ['create', 'render', 'dispose'])
  assert.equal(h.frames.size, 0)
})

test('event callbacks can change without rebinding and removed events are released', () => {
  const active = new Map()
  const binds = []
  const unbinds = []
  const received = []
  const bindings = createChartEventBindings({
    on(name, handler) { active.set(name, handler); binds.push(name) },
    off(name, handler) { assert.equal(active.get(name), handler); active.delete(name); unbinds.push(name) },
  })
  bindings.update({})
  bindings.update({ click: (value) => received.push(['first', value]) })
  const click = active.get('click')
  bindings.update({ click: (value) => received.push(['latest', value]) })
  click(7)
  assert.deepEqual(binds, ['click'])
  assert.deepEqual(unbinds, [])
  assert.deepEqual(received, [['latest', 7]])
  bindings.update({ datazoom: () => {} })
  assert.deepEqual(unbinds, ['click'])
  click(8)
  assert.equal(received.length, 1)
  bindings.dispose()
  bindings.update({ click: () => {} })
  bindings.dispose()
  assert.equal(active.size, 0)
  assert.deepEqual(binds, ['click', 'datazoom'])
  assert.deepEqual(unbinds, ['click', 'datazoom'])
})
