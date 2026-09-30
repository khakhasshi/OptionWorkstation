import { Plus } from 'lucide-react'
import { memo, useCallback, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { CHAIN_HEADER_HEIGHT, CHAIN_ROW_HEIGHT, chainWindow, groupChainRows, scrollToChainRow } from '../lib/chainTable'

const countFormatter = new Intl.NumberFormat()

const OptionCells = memo(function OptionCells({ row, side, onAdd, oiReady }) {
  if (!row) return side === 'put'
    ? <><td></td><td colSpan="5" className="empty-contract">--</td></>
    : <><td colSpan="5" className="empty-contract">--</td><td></td></>
  const cells = [
    <td key="delta">{row.delta.toFixed(3)}</td>,
    <td key="iv">{row.iv.toFixed(1)}%</td>,
    <td key="bid">{row.bid.toFixed(2)}</td>,
    <td key="ask">{row.ask.toFixed(2)}</td>,
    <td key="oi">{oiReady === false ? '--' : countFormatter.format(row.open_interest)}</td>,
  ]
  const label = `添加 ${row.strike} ${row.right}`
  const add = <td key="add" className="chain-action-cell"><button className="add-leg" title={label} aria-label={label} onClick={(event) => { event.stopPropagation(); onAdd(row) }}><Plus size={12} /></button></td>
  return <>{side === 'put' ? [add, ...cells.slice().reverse()] : [...cells, add]}</>
})

const tableHead = <thead><tr><th colSpan="6" className="call-group">CALL</th><th className="strike-group">STRIKE</th><th colSpan="6" className="put-group">PUT</th></tr><tr><th>Delta</th><th>IV</th><th>Bid</th><th>Ask</th><th>OI</th><th aria-label="添加看涨腿"></th><th className="strike-group">Strike</th><th aria-label="添加看跌腿"></th><th>OI</th><th>Ask</th><th>Bid</th><th>IV</th><th>Delta</th></tr></thead>

function ChainTable({ chain, onAdd, onFocus, focusStrike }) {
  const [qualityFloor, setQualityFloor] = useState(50)
  const [maxSpread, setMaxSpread] = useState(35)
  const [viewport, setViewport] = useState({ scrollTop: 0, height: 320 })
  const [activeStrike, setActiveStrike] = useState(null)
  const scrollerRef = useRef(null)
  const scrollFrameRef = useRef(null)
  const pendingFocusRef = useRef(null)
  const tableFocusRef = useRef(null)
  const rows = useMemo(() => groupChainRows(chain, qualityFloor, maxSpread), [chain, qualityFloor, maxSpread])
  const rowWindow = chainWindow(rows.length, viewport.scrollTop, viewport.height)
  const activeIndex = rows.findIndex((row) => row.strike === activeStrike)
  const tabIndex = activeIndex >= rowWindow.start && activeIndex < rowWindow.end ? activeIndex : rowWindow.firstVisible

  const measure = useCallback(() => {
    const scroller = scrollerRef.current
    if (!scroller) return
    const next = { scrollTop: scroller.scrollTop, height: scroller.clientHeight }
    setViewport((current) => current.scrollTop === next.scrollTop && current.height === next.height ? current : next)
  }, [])

  useLayoutEffect(() => {
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(scrollerRef.current)
    return () => {
      observer.disconnect()
      if (scrollFrameRef.current !== null) cancelAnimationFrame(scrollFrameRef.current)
    }
  }, [measure])

  useLayoutEffect(() => {
    // A narrower filter may reduce the scroll range before the browser clamps it.
    const scroller = scrollerRef.current
    if (scroller) {
      const clamped = chainWindow(rows.length, scroller.scrollTop, viewport.height).scrollTop
      if (scroller.scrollTop !== clamped) scroller.scrollTop = clamped
      if (viewport.scrollTop !== rowWindow.scrollTop || scroller.scrollTop !== viewport.scrollTop) measure()
    }
    const index = pendingFocusRef.current
    if (index !== null) {
      const row = scroller?.querySelector(`[data-row-index="${index}"]`)
      if (row) {
        row.focus({ preventScroll: true })
        pendingFocusRef.current = null
      }
    }
  }, [rowWindow.scrollTop, rowWindow.start, rowWindow.end, rows.length, activeStrike, measure])

  useLayoutEffect(() => {
    const previous = tableFocusRef.current
    const scroller = scrollerRef.current
    // A quote refresh can remove the focused strike. Restore only focus lost
    // from the table itself, never focus now held by a filter or another panel.
    if (!previous || !scroller || pendingFocusRef.current !== null || document.activeElement !== document.body) return
    if (!rows.length) {
      tableFocusRef.current = null
      return
    }
    const expectedTop = chainWindow(rows.length, previous.scrollTop, viewport.height).scrollTop
    if (Math.abs(scroller.scrollTop - expectedTop) > 1) {
      tableFocusRef.current = null
      return
    }
    const matchingIndex = rows.findIndex((row) => row.strike === previous.strike)
    const next = matchingIndex < 0 ? Math.min(previous.index, rows.length - 1) : matchingIndex
    const node = scroller.querySelector(`[data-row-index="${next}"]`)
    setActiveStrike(rows[next].strike)
    if (node) node.focus({ preventScroll: true })
    else {
      pendingFocusRef.current = next
      scroller.scrollTop = scrollToChainRow(next, rows.length, scroller.scrollTop, viewport.height)
      measure()
    }
  }, [rows, measure])

  const onScroll = () => {
    if (scrollFrameRef.current !== null) return
    scrollFrameRef.current = requestAnimationFrame(() => {
      scrollFrameRef.current = null
      measure()
    })
  }

  const onRowKeyDown = (event, index) => {
    if (event.target !== event.currentTarget) return
    if (event.key === 'Enter' || event.key === ' ') {
      event.preventDefault()
      onFocus(rows[index].strike)
      return
    }
    const page = Math.max(1, Math.floor((viewport.height - CHAIN_HEADER_HEIGHT) / CHAIN_ROW_HEIGHT))
    const destinations = { ArrowDown: index + 1, ArrowUp: index - 1, Home: 0, End: rows.length - 1, PageDown: index + page, PageUp: index - page }
    if (!(event.key in destinations)) return
    event.preventDefault()
    const next = Math.max(0, Math.min(rows.length - 1, destinations[event.key]))
    if (next === index) {
      pendingFocusRef.current = null
      return
    }
    pendingFocusRef.current = next
    setActiveStrike(rows[next].strike)
    onFocus(rows[next].strike)
    scrollerRef.current.scrollTop = scrollToChainRow(next, rows.length, viewport.scrollTop, viewport.height)
    measure()
  }

  return <div className="chain-shell">
    <div className="chain-filters">
      <span>质量</span><div className="segments mini">{[0, 50, 70].map((value) => <button key={value} aria-pressed={qualityFloor === value} className={qualityFloor === value ? 'active' : ''} onClick={() => setQualityFloor(value)}>{value}+</button>)}</div>
      <label><span>最大价差 %</span><input type="number" min="1" max="100" value={maxSpread} onChange={(event) => setMaxSpread(Number(event.target.value))} /></label>
      <b>{rows.length} strikes</b>
    </div>
    <div className="chain-table mirrored virtual-chain" ref={scrollerRef} onScroll={onScroll} onFocusCapture={(event) => {
      const row = event.target.closest('.chain-data-row')
      if (row) {
        const index = Number(row.dataset.rowIndex)
        tableFocusRef.current = { index, strike: rows[index].strike, scrollTop: event.currentTarget.scrollTop }
      }
    }} onBlurCapture={(event) => {
      if (!event.currentTarget.contains(event.relatedTarget)) tableFocusRef.current = null
    }}>
      <table className="virtual-chain-table" aria-label="期权链，方向键移动行，回车选择行权价" aria-rowcount={rows.length + 2}>
        <colgroup>{Array.from({ length: 13 }, (_, index) => <col key={index} style={index === 5 || index === 7 ? { width: 30 } : index === 6 ? { width: 80 } : undefined} />)}</colgroup>
        {tableHead}
        <tbody>
          {rowWindow.paddingTop > 0 && <tr className="chain-spacer" aria-hidden="true"><td colSpan="13" style={{ height: rowWindow.paddingTop }}></td></tr>}
          {rows.slice(rowWindow.start, rowWindow.end).map((row, offset) => {
            const index = rowWindow.start + offset
            return <tr className={`chain-data-row${focusStrike === row.strike ? ' focused' : ''}`} key={row.strike} data-row-index={index} aria-rowindex={index + 3} aria-selected={focusStrike === row.strike} tabIndex={index === tabIndex ? 0 : -1} onKeyDown={(event) => onRowKeyDown(event, index)} onClick={() => { setActiveStrike(row.strike); onFocus(row.strike) }}>
              <OptionCells row={row.call} side="call" onAdd={onAdd} oiReady={chain?.quality?.gex_ready} />
              <td className="strike-cell">{row.strike}</td>
              <OptionCells row={row.put} side="put" onAdd={onAdd} oiReady={chain?.quality?.gex_ready} />
            </tr>
          })}
          {rowWindow.paddingBottom > 0 && <tr className="chain-spacer" aria-hidden="true"><td colSpan="13" style={{ height: rowWindow.paddingBottom }}></td></tr>}
          {rows.length === 0 && <tr className="chain-empty-row"><td colSpan="13" className="empty-contract">{chain ? '暂无符合筛选条件的期权' : '暂无期权数据'}</td></tr>}
        </tbody>
      </table>
    </div>
  </div>
}

export default memo(ChainTable)
