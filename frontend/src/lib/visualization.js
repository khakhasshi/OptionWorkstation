export const WORKSPACE_VIEWS = {
  dense: { label: '总览', description: '行情、波动率与持仓暴露', panels: new Set(['market', 'snapshot', 'vol', 'smile', 'exposure']) },
  vol: { label: '波动率', description: '曲面、微笑与拟合诊断', panels: new Set(['snapshot', 'vol', 'surface', 'smile', 'residual', 'term']) },
  trade: { label: '交易', description: '期权链、组合与风险预览', panels: new Set(['market', 'snapshot', 'strategy', 'chain', 'exposure']) },
  audit: { label: '记录', description: '研究快照与纸面执行记录', panels: new Set(['snapshot', 'audit', 'execution']) },
}

export function chartRange(grid, fixed = false, observed = []) {
  if (fixed) return { min: 0, max: 150 }
  let low = Infinity
  let high = -Infinity
  const include = (value) => {
    if (!Number.isFinite(value) || value < 0) return
    low = Math.min(low, value)
    high = Math.max(high, value)
  }
  for (const row of grid || []) for (const cell of row) include(cell[2])
  for (const point of observed) include(point.iv)
  if (!Number.isFinite(low)) return { min: 0, max: 100 }
  const padding = Math.max(2, (high - low) * 0.08)
  return { min: Math.max(0, Math.floor((low - padding) / 5) * 5), max: Math.ceil((high + padding) / 5) * 5 }
}

export function exposureSeries(chain) {
  if (!chain?.quality?.gex_ready) return []
  const grouped = new Map()
  for (const row of chain.rows || []) {
    if (Math.abs(row.strike / chain.spot - 1) > 0.12) continue
    const value = grouped.get(row.strike) || { strike: row.strike, gex: 0, vanna: 0, charm: 0 }
    const sign = chain.dealer_model === 'long_all' ? 1 : chain.dealer_model === 'short_all' ? -1 : row.right === 'CALL' ? 1 : -1
    value.gex += row.gex || 0
    value.vanna += row.vanna * row.open_interest * 100 * sign
    value.charm += row.charm * row.open_interest * 100 * sign
    grouped.set(row.strike, value)
  }
  return [...grouped.values()].sort((left, right) => left.strike - right.strike)
}
