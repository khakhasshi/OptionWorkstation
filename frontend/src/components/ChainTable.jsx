import { Plus } from 'lucide-react'
import { useMemo, useState } from 'react'

function OptionCells({ row, side, onAdd }) {
  if (!row) return <><td colSpan="5" className="empty-contract">--</td><td></td></>
  const cells = [
    <td key="delta">{row.delta.toFixed(3)}</td>,
    <td key="iv">{row.iv.toFixed(1)}%</td>,
    <td key="bid">{row.bid.toFixed(2)}</td>,
    <td key="ask">{row.ask.toFixed(2)}</td>,
    <td key="oi">{row.open_interest_ready === false ? '--' : row.open_interest.toLocaleString()}</td>,
  ]
  const add = <td key="add"><button className="add-leg" title={`添加 ${row.right}`} onClick={(event) => { event.stopPropagation(); onAdd(row) }}><Plus size={12} /></button></td>
  return <>{side === 'put' ? [add, ...cells.slice().reverse()] : [...cells, add]}</>
}

export default function ChainTable({ chain, onAdd, onFocus, focusStrike }) {
  const [qualityFloor, setQualityFloor] = useState(50)
  const [maxSpread, setMaxSpread] = useState(35)
  const rows = useMemo(() => {
    if (!chain) return []
    const byStrike = new Map()
    chain.rows
      .filter((row) => Math.abs(row.strike / chain.spot - 1) <= 0.15)
      .filter((row) => row.quality_score >= qualityFloor && row.spread_pct <= maxSpread)
      .forEach((row) => {
        const current = byStrike.get(row.strike) || { strike: row.strike }
        current[row.right.toLowerCase()] = { ...row, open_interest_ready: chain.quality?.gex_ready }
        byStrike.set(row.strike, current)
      })
    return [...byStrike.values()].sort((left, right) => left.strike - right.strike)
  }, [chain, qualityFloor, maxSpread])

  return <div className="chain-shell">
    <div className="chain-filters">
      <span>质量</span><div className="segments mini">{[0, 50, 70].map((value) => <button key={value} className={qualityFloor === value ? 'active' : ''} onClick={() => setQualityFloor(value)}>{value}+</button>)}</div>
      <label><span>最大价差 %</span><input type="number" min="1" max="100" value={maxSpread} onChange={(event) => setMaxSpread(Number(event.target.value))} /></label>
      <b>{rows.length} strikes</b>
    </div>
    <div className="chain-table mirrored"><table>
      <thead><tr><th colSpan="6" className="call-group">CALL</th><th className="strike-group">STRIKE</th><th colSpan="6" className="put-group">PUT</th></tr><tr><th>Delta</th><th>IV</th><th>Bid</th><th>Ask</th><th>OI</th><th></th><th>Strike</th><th></th><th>OI</th><th>Ask</th><th>Bid</th><th>IV</th><th>Delta</th></tr></thead>
      <tbody>{rows.map((row) => <tr className={focusStrike === row.strike ? 'focused' : ''} key={row.strike} onClick={() => onFocus(row.strike)}>
        <OptionCells row={row.call} side="call" onAdd={onAdd} />
        <td className="strike-cell">{row.strike}</td>
        <OptionCells row={row.put} side="put" onAdd={onAdd} />
      </tr>)}</tbody>
    </table></div>
  </div>
}
