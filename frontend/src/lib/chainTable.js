export const CHAIN_ROW_HEIGHT = 32
export const CHAIN_HEADER_HEIGHT = 64
const OVERSCAN = 6

export function groupChainRows(chain, qualityFloor, maxSpread) {
  if (!chain?.rows || !Number.isFinite(chain.spot) || chain.spot <= 0) return []
  const byStrike = new Map()
  for (const row of chain.rows) {
    if (!(Math.abs(row.strike / chain.spot - 1) <= 0.15
      && row.quality_score >= qualityFloor && row.spread_pct <= maxSpread)) continue
    const side = row.right === 'CALL' ? 'call' : row.right === 'PUT' ? 'put' : null
    if (!side) continue
    let pair = byStrike.get(row.strike)
    if (!pair) {
      pair = { strike: row.strike }
      byStrike.set(row.strike, pair)
    }
    pair[side] = row
  }
  return [...byStrike.values()].sort((left, right) => left.strike - right.strike)
}

export function chainWindow(rowCount, scrollTop, viewportHeight) {
  const count = Math.max(0, Math.floor(rowCount))
  const bodyHeight = Math.max(CHAIN_ROW_HEIGHT, viewportHeight - CHAIN_HEADER_HEIGHT)
  const maxScrollTop = Math.max(0, count * CHAIN_ROW_HEIGHT - bodyHeight)
  const top = Math.min(Math.max(0, scrollTop), maxScrollTop)
  const firstVisible = Math.min(Math.floor(top / CHAIN_ROW_HEIGHT), count)
  const endVisible = Math.min(Math.ceil((top + bodyHeight) / CHAIN_ROW_HEIGHT), count)
  const start = Math.max(0, firstVisible - OVERSCAN)
  const end = Math.min(count, endVisible + OVERSCAN)
  return {
    start, end, firstVisible, scrollTop: top, maxScrollTop,
    paddingTop: start * CHAIN_ROW_HEIGHT,
    paddingBottom: (count - end) * CHAIN_ROW_HEIGHT,
  }
}

export function scrollToChainRow(rowIndex, rowCount, scrollTop, viewportHeight) {
  const window = chainWindow(rowCount, scrollTop, viewportHeight)
  if (!rowCount) return 0
  const index = Math.max(0, Math.min(rowCount - 1, rowIndex))
  const bodyHeight = Math.max(CHAIN_ROW_HEIGHT, viewportHeight - CHAIN_HEADER_HEIGHT)
  const rowTop = index * CHAIN_ROW_HEIGHT
  const rowBottom = rowTop + CHAIN_ROW_HEIGHT
  const nextTop = rowTop < window.scrollTop ? rowTop
    : rowBottom > window.scrollTop + bodyHeight ? rowBottom - bodyHeight : window.scrollTop
  return Math.min(nextTop, window.maxScrollTop)
}
