import { RefreshCw, Send, ShieldCheck, X } from 'lucide-react'
import { useMemo, useState } from 'react'
import Chart from './Chart'
import { Metric } from './Primitives'

function money(value) {
  if (value == null) return '∞'
  return `${value < 0 ? '-' : ''}$${Math.abs(value).toFixed(0)}`
}

export default function StrategyWorkbench({
  legs,
  setLegs,
  onPreset,
  analysis,
  quantity,
  setQuantity,
  onAnalyze,
  payoffOption,
  tradeAccount,
  onPaperSubmit,
  analyzing,
  paperEligible,
}) {
  const [elapsed, setElapsed] = useState(0)
  const update = (index, patch) => setLegs((current) => current.map((leg, legIndex) => legIndex === index ? { ...leg, ...patch } : leg))
  const scenario = useMemo(() => {
    const points = analysis?.scenarios?.filter((point) => point.elapsed_fraction === elapsed) || []
    return {
      spots: [...new Set(points.map((point) => point.spot_shock_pct))],
      ivs: [...new Set(points.map((point) => point.iv_shock_points))],
      points,
    }
  }, [analysis, elapsed])
  const scenarioValue = (iv, spot) => scenario.points.find((point) => point.iv_shock_points === iv && point.spot_shock_pct === spot)?.pnl

  return <div className="strategy-workbench">
    <div className="strategy-legs">
      <div className="preset-strip"><button onClick={() => onPreset('straddle')}>跨式</button><button onClick={() => onPreset('strangle')}>宽跨</button><button onClick={() => onPreset('bull_call')}>牛市价差</button><button onClick={() => onPreset('iron_condor')}>铁鹰</button></div>
      <div className="strategy-toolbar"><label><span>组合数量</span><input type="number" min="1" max="20" value={quantity} onChange={(event) => setQuantity(Math.max(1, Number(event.target.value)))} /></label><button onClick={onAnalyze} disabled={!legs.length || analyzing}>{analyzing ? <RefreshCw className="spin" size={13} /> : <ShieldCheck size={13} />}风险预览</button></div>
      {legs.length ? legs.map((leg, index) => <div className="strategy-leg" key={`${leg.strike}-${leg.right}`}>
        <button className={leg.side === 'BUY' ? 'buy' : 'sell'} onClick={() => update(index, { side: leg.side === 'BUY' ? 'SELL' : 'BUY' })}>{leg.side}</button>
        <b>{leg.right} {leg.strike}</b>
        <input type="number" min="1" max="20" value={leg.ratio} onChange={(event) => update(index, { ratio: Math.max(1, Number(event.target.value)) })} />
        <span>{leg.bid?.toFixed(2) ?? '--'} / {leg.ask?.toFixed(2) ?? '--'}</span>
        <X size={14} onClick={() => setLegs((current) => current.filter((_, legIndex) => legIndex !== index))} />
      </div>) : <div className="empty-state">从镜像期权链添加策略腿</div>}
      {analysis && <>
        <div className="risk-metrics">
          <Metric label="Entry Cash" value={money(analysis.entry_cash_flow)} />
          <Metric label="Max Profit" value={money(analysis.max_profit)} />
          <Metric label="Max Loss" value={money(analysis.max_loss)} />
          <Metric label="Margin" value={money(analysis.margin_estimate)} />
          <Metric label="POP" value={analysis.probability_of_profit == null ? '--' : `${analysis.probability_of_profit.toFixed(1)}%`} />
          <Metric label="B/E" value={analysis.break_evens?.map((value) => value.toFixed(1)).join(' · ') || '--'} />
        </div>
        {analysis.blockers?.length > 0 && <div className="risk-blockers">{analysis.blockers.map((blocker) => <span key={blocker}>{blocker}</span>)}</div>}
        <button className="paper-submit" disabled={!analysis.executable || !tradeAccount?.execution_enabled || !paperEligible} onClick={onPaperSubmit}><Send size={13} />{!paperEligible ? '仅实时模式可提交' : tradeAccount?.execution_enabled ? '提交纸面组合' : '纸面执行已锁定'}</button>
      </>}
    </div>
    <div className="strategy-visuals">
      <Chart option={payoffOption} />
      {analysis?.scenarios?.length > 0 && <div className="scenario-matrix">
        <div className="scenario-tabs">{[0, 0.5, 0.9].map((value) => <button key={value} className={elapsed === value ? 'active' : ''} onClick={() => setElapsed(value)}>T {Math.round(value * 100)}%</button>)}</div>
        <table><thead><tr><th>IV / Spot</th>{scenario.spots.map((spot) => <th key={spot}>{spot > 0 ? '+' : ''}{spot}%</th>)}</tr></thead><tbody>{scenario.ivs.map((iv) => <tr key={iv}><th>{iv > 0 ? '+' : ''}{iv}pt</th>{scenario.spots.map((spot) => { const value = scenarioValue(iv, spot); return <td className={value >= 0 ? 'scenario-up' : 'scenario-down'} key={spot}>{money(value)}</td> })}</tr>)}</tbody></table>
      </div>}
    </div>
  </div>
}
