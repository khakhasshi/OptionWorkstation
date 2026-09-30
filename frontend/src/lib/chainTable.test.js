import assert from 'node:assert/strict'
import test from 'node:test'
import { CHAIN_ROW_HEIGHT, chainWindow, groupChainRows, scrollToChainRow } from './chainTable.js'

const quote = (strike, right, extra = {}) => ({ strike, right, quality_score: 70, spread_pct: 10, open_interest: 0, ...extra })

test('chain grouping keeps mirror pairs, sort order and last quote without mutating metadata', () => {
  const call = Object.freeze(quote(100, 'CALL'))
  const put = Object.freeze(quote(100, 'PUT', { open_interest: 25 }))
  const latest = Object.freeze(quote(100, 'CALL', { bid: 2.2 }))
  const lower = Object.freeze(quote(95, 'PUT'))
  const rows = groupChainRows({ spot: 100, quality: { gex_ready: false }, rows: [put, call, lower, latest] }, 50, 35)
  assert.deepEqual(rows.map((row) => row.strike), [95, 100])
  assert.equal(rows[0].call, undefined)
  assert.equal(rows[0].put, lower)
  assert.equal(rows[1].call, latest)
  assert.equal(rows[1].put, put)
  assert.equal(rows[1].call.open_interest, 0)
  assert.equal(Object.hasOwn(latest, 'open_interest_ready'), false)
})

test('chain filters reject outliers and invalid quality while preserving threshold matches', () => {
  const eligible = quote(100, 'CALL', { quality_score: 50, spread_pct: 35 })
  const rows = groupChainRows({ spot: 100, rows: [
    eligible,
    quote(120, 'CALL'),
    quote(101, 'PUT', { quality_score: 49 }),
    quote(102, 'CALL', { spread_pct: 36 }),
    quote(103, 'CALL', { quality_score: NaN }),
    quote(NaN, 'CALL'),
    quote(104, 'PUT', { spread_pct: NaN }),
  ] }, 50, 35)
  assert.deepEqual(rows, [{ strike: 100, call: eligible }])
  assert.deepEqual(groupChainRows(null, 50, 35), [])
  assert.deepEqual(groupChainRows({ spot: 0, rows: [eligible] }, 50, 35), [])
  assert.deepEqual(groupChainRows({ spot: 100, rows: [eligible] }, 70, 35), [])
})

test('large chain windows bound mounted rows while covering every visible strike', () => {
  const top = chainWindow(10_000, 0, 320)
  assert.deepEqual([top.start, top.end, top.paddingTop], [0, 14, 0])
  const middle = chainWindow(10_000, 3_205, 320)
  assert.deepEqual([middle.start, middle.end, middle.firstVisible], [94, 115, 100])
  const bottom = chainWindow(10_000, 1_000_000, 320)
  assert.deepEqual([bottom.start, bottom.end, bottom.scrollTop], [9986, 10_000, 319_744])
  for (const window of [top, middle, bottom]) {
    assert.ok(window.end - window.start <= 21)
    assert.ok(window.start <= window.firstVisible)
    assert.equal(window.paddingTop + (window.end - window.start) * CHAIN_ROW_HEIGHT + window.paddingBottom, 320_000)
  }
})

test('filter shrink and viewport growth clamp scroll without leaving a blank table', () => {
  const before = chainWindow(150, 4_000, 320)
  assert.equal(before.scrollTop, 4_000)
  const filtered = chainWindow(3, before.scrollTop, 320)
  assert.deepEqual([filtered.scrollTop, filtered.start, filtered.end, filtered.paddingBottom], [0, 0, 3, 0])
  const grown = chainWindow(20, 384, 704)
  assert.deepEqual([grown.scrollTop, grown.start, grown.end], [0, 0, 20])
  const empty = chainWindow(0, 4_000, 320)
  assert.deepEqual([empty.scrollTop, empty.start, empty.end, empty.paddingTop, empty.paddingBottom], [0, 0, 0, 0, 0])
  assert.equal(chainWindow(100, -100, 320).scrollTop, 0)
})

test('keyboard navigation reveals rows beyond the window without moving already visible rows', () => {
  assert.equal(scrollToChainRow(7, 100, 0, 320), 0)
  assert.equal(scrollToChainRow(8, 100, 0, 320), 32)
  assert.equal(scrollToChainRow(99, 100, 0, 320), 2_944)
  assert.equal(scrollToChainRow(0, 100, 2_944, 320), 0)
  assert.equal(scrollToChainRow(50, 100, 1_600, 320), 1_600)
  assert.equal(scrollToChainRow(-10, 100, 1_600, 320), 0)
  assert.equal(scrollToChainRow(1000, 100, 0, 320), 2_944)
  assert.equal(scrollToChainRow(0, 0, 1_600, 320), 0)
})
