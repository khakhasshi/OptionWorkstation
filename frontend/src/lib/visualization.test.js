import assert from 'node:assert/strict'
import test from 'node:test'
import { chartRange, exposureSeries } from './visualization.js'

test('surface auto range contains all displayed observations, including extremes and flat slices', () => {
  for (const grid of [[], [[[1, 7, 22]]], [[[1, 7, 22], [1.1, 7, 24]]], [[[1, 7, 180]]]]) {
    const range = chartRange(grid)
    assert.ok(range.min >= 0 && range.max > range.min)
    for (const row of grid) for (const cell of row) assert.ok(cell[2] >= range.min && cell[2] <= range.max)
  }
  assert.equal(chartRange([[[1, 7, NaN]]]).max, 100)
  assert.ok(chartRange([[[1, 7, 20]]], false, [{ iv: 240 }]).max >= 240)
  assert.deepEqual(chartRange([[[1, 7, 240]]], true), { min: 0, max: 150 })
})

test('exposure separates quantities, preserves the OI gate and follows the snapshot dealer convention', () => {
  const chain = { spot: 100, quality: { gex_ready: true }, rows: [
    { strike: 100, right: 'PUT', gex: -20, vanna: 0.2, charm: 0.1, open_interest: 2 },
    { strike: 100, right: 'CALL', gex: 30, vanna: 0.3, charm: 0.2, open_interest: 3 },
    { strike: 150, right: 'CALL', gex: 900, vanna: 1, charm: 1, open_interest: 100 },
  ] }
  const [classic] = exposureSeries(chain)
  assert.equal(classic.strike, 100)
  assert.equal(classic.gex, 10)
  assert.ok(Math.abs(classic.vanna - 50) < 1e-10)
  assert.ok(Math.abs(classic.charm - 40) < 1e-10)
  assert.ok(Math.abs(exposureSeries({ ...chain, dealer_model: 'long_all' })[0].vanna - 130) < 1e-10)
  assert.ok(Math.abs(exposureSeries({ ...chain, dealer_model: 'short_all' })[0].vanna + 130) < 1e-10)
  assert.deepEqual(exposureSeries({ ...chain, quality: { gex_ready: false } }), [])
})
