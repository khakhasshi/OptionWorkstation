import { lazy, Suspense, useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  Activity,
  BarChart3,
  Bookmark,
  BookOpen,
  ChevronLeft,
  ChevronRight,
  Download,
  Gauge,
  History,
  Layers3,
  KeyRound,
  LockKeyhole,
  Pause,
  Play,
  Plus,
  Radio,
  RefreshCw,
  RotateCcw,
  TableProperties,
  Wifi,
  WifiOff,
  X,
} from 'lucide-react'
import AuditPanel from './components/AuditPanel'
import Chart from './components/Chart'
import ChainTable from './components/ChainTable'
import ExecutionPanel from './components/ExecutionPanel'
import { LiveReadout, Metric, Panel } from './components/Primitives'
import StrategyWorkbench from './components/StrategyWorkbench'
import { api, apiJson, websocketUrl } from './lib/api'

const SurfaceChart = lazy(() => import('./components/SurfaceChart'))
const PALETTE = ['#54d6b6', '#70a5ff', '#f1c75b', '#ff7e8a', '#b395ff']
const SPEEDS = [0.5, 1, 2, 5, 10, 30]

function detectWebGL() {
  try {
    const canvas = document.createElement('canvas')
    return Boolean(canvas.getContext('webgl') || canvas.getContext('experimental-webgl'))
  } catch {
    return false
  }
}

const axis = {
  axisLine: { lineStyle: { color: '#34414d' } },
  axisTick: { show: false },
  axisLabel: { color: '#83909c', fontSize: 11 },
  splitLine: { lineStyle: { color: '#1d2730' } },
}

function formatCompact(value) {
  if (value == null) return '--'
  const absolute = Math.abs(value)
  if (absolute >= 1e9) return `${(value / 1e9).toFixed(2)}B`
  if (absolute >= 1e6) return `${(value / 1e6).toFixed(2)}M`
  if (absolute >= 1e3) return `${(value / 1e3).toFixed(1)}K`
  return Number(value).toFixed(2)
}

function App() {
  const [mode, setMode] = useState(() => new URLSearchParams(window.location.search).get('mode') === 'live' ? 'live' : 'replay')
  const [catalog, setCatalog] = useState(null)
  const [symbols, setSymbols] = useState(['SPY'])
  const [activeSymbol, setActiveSymbol] = useState('SPY')
  const [tradingDate, setTradingDate] = useState('')
  const [session, setSession] = useState(null)
  const [frame, setFrame] = useState(0)
  const [playing, setPlaying] = useState(false)
  const [speed, setSpeed] = useState(2)
  const [expiration, setExpiration] = useState('')
  const [chain, setChain] = useState(null)
  const [surface, setSurface] = useState(null)
  const [volContext, setVolContext] = useState(null)
  const [pricingMode, setPricingMode] = useState('micro')
  const [dealerModel, setDealerModel] = useState('classic')
  const [smileAxis, setSmileAxis] = useState('strike')
  const [focusStrike, setFocusStrike] = useState(null)
  const [strategyLegs, setStrategyLegs] = useState([])
  const [strategyQuantity, setStrategyQuantity] = useState(1)
  const [strategyAnalysis, setStrategyAnalysis] = useState(null)
  const [analyzing, setAnalyzing] = useState(false)
  const [auditRecords, setAuditRecords] = useState([])
  const [compareBookmark, setCompareBookmark] = useState(null)
  const [compareChain, setCompareChain] = useState(null)
  const [webgl] = useState(detectWebGL)
  const [layout, setLayout] = useState(() => localStorage.getItem('option-workstation-layout') || 'dense')
  const [loading, setLoading] = useState(false)
  const [error, setError] = useState('')
  const [connection, setConnection] = useState({ connected: false, state: 'disconnected', packages: [], subscribed_contracts: 0 })
  const [credentialOpen, setCredentialOpen] = useState(false)
  const [credentials, setCredentials] = useState({ app_key: '', app_secret: '', access_token: '' })
  const [liveSymbolDraft, setLiveSymbolDraft] = useState('SPY')
  const [liveFeed, setLiveFeed] = useState(null)
  const [liveSocketState, setLiveSocketState] = useState('idle')
  const [liveSwitch, setLiveSwitch] = useState(null)
  const [liveSettings, setLiveSettings] = useState({ max_contracts: 420, surface_expiries: 4, moneyness_window: 0.12 })
  const [tradeAccount, setTradeAccount] = useState(null)
  const [orders, setOrders] = useState([])
  const [paperConfirmOpen, setPaperConfirmOpen] = useState(false)
  const [paperConfirmation, setPaperConfirmation] = useState('')
  const surfaceUpdateRef = useRef({ at: 0, key: '' })
  const liveSequenceRef = useRef(-1)
  const pendingLiveSymbolRef = useRef(null)
  const liveRequestRef = useRef({ id: 0, controller: null, timer: null })

  const refreshAudit = useCallback(async () => {
    const records = await api('/api/audit/records?limit=50')
    setAuditRecords(records)
    return records
  }, [])

  const refreshTrading = useCallback(async () => {
    if (!connection.connected || !connection.trade_connected) {
      setTradeAccount(null)
      setOrders([])
      return
    }
    try {
      const [account, todayOrders] = await Promise.all([
        api('/api/trade/account'),
        api('/api/trade/orders'),
      ])
      setTradeAccount(account)
      setOrders(Array.isArray(todayOrders) ? todayOrders : [])
    } catch (reason) {
      setTradeAccount(null)
      setOrders([])
      setError(reason.message)
    }
  }, [connection.connected, connection.trade_connected])

  useEffect(() => {
    Promise.all([api('/api/catalog'), api('/api/connection')]).then(([data, status]) => {
      setCatalog(data)
      setConnection(status)
      setTradingDate(data.common_dates.at(-1) || '')
    }).catch((reason) => setError(reason.message))
    refreshAudit().catch((reason) => setError(reason.message))
  }, [refreshAudit])

  useEffect(() => {
    localStorage.setItem('option-workstation-layout', layout)
  }, [layout])

  useEffect(() => {
    refreshTrading()
  }, [refreshTrading])

  const applyLiveSnapshot = useCallback((data, force = false) => {
    if (data.kind === 'live_error') {
      setError(data.detail || '实时行情更新失败')
      return
    }
    const sequence = Number(data.sequence)
    if (!force && Number.isFinite(sequence) && sequence < liveSequenceRef.current) return
    if (Number.isFinite(sequence)) liveSequenceRef.current = sequence
    const bars = data.bars || []
    const symbol = data.feed.symbol
    const timeline = bars.map((bar) => bar.time)
    setLiveFeed(data.feed)
    setActiveSymbol(symbol)
    setSymbols([symbol])
    setExpiration(data.feed.expiration)
    setTradingDate(data.chain.date)
    setChain(data.chain)
    const surfaceKey = `${symbol}:${data.feed.expiration}:${data.feed.subscribed_contracts}:${data.feed.expirations.join(',')}`
    const now = Date.now()
    if (surfaceUpdateRef.current.key !== surfaceKey || now - surfaceUpdateRef.current.at >= 2000) {
      setSurface(data.surface)
      surfaceUpdateRef.current = { at: now, key: surfaceKey }
    }
    setVolContext((current) => current?.symbol === symbol ? {
      ...current,
      as_of: data.chain.timestamp,
      atm_iv: data.chain.metrics.atm_iv,
      tte_years: data.chain.tte_years,
      expected_move: data.chain.metrics.atm_iv == null ? null : data.chain.spot * data.chain.metrics.atm_iv / 100 * Math.sqrt(data.chain.tte_years),
    } : current)
    setSession({
      date: data.chain.date,
      symbols: [symbol],
      timeline,
      series: { [symbol]: { bars, expirations: data.feed.expirations } },
    })
    setFrame(Math.max(0, timeline.length - 1))
    setConnection((current) => ({ ...current, state: 'streaming', subscribed_contracts: data.feed.subscribed_contracts, last_event_at: data.feed.as_of }))
    if (!pendingLiveSymbolRef.current) setError('')
  }, [])

  useEffect(() => {
    if (mode !== 'live' || !connection.connected || liveFeed) return
    api('/api/live/snapshot')
      .then(applyLiveSnapshot)
      .catch((reason) => {
        if (!reason.message.includes('尚未建立')) setError(reason.message)
      })
  }, [mode, connection.connected, Boolean(liveFeed), applyLiveSnapshot])

  const switchMode = (nextMode) => {
    if (nextMode === mode) return
    const pending = liveRequestRef.current
    pending.controller?.abort()
    window.clearTimeout(pending.timer)
    liveRequestRef.current = { id: pending.id + 1, controller: null, timer: null }
    pendingLiveSymbolRef.current = null
    setLiveSwitch(null)
    setMode(nextMode)
    setPlaying(false)
    setSession(null)
    setChain(null)
    setSurface(null)
    surfaceUpdateRef.current = { at: 0, key: '' }
    setVolContext(null)
    setFocusStrike(null)
    setError('')
    if (nextMode === 'replay') {
      setTradingDate(catalog?.common_dates.at(-1) || '')
      setExpiration('')
    } else {
      setSymbols([activeSymbol])
      setLiveSymbolDraft(activeSymbol)
      setTradingDate('')
      setExpiration(liveFeed?.symbol === activeSymbol ? liveFeed.expiration : '')
    }
    const url = new URL(window.location.href)
    url.searchParams.set('mode', nextMode)
    window.history.replaceState({}, '', url)
  }

  const submitCredentials = async (event) => {
    event.preventDefault()
    setLoading(true)
    setError('')
    try {
      const status = await apiJson('/api/connection', 'POST', credentials)
      setConnection(status)
      setCredentials({ app_key: '', app_secret: '', access_token: '' })
      setCredentialOpen(false)
    } catch (reason) {
      setError(reason.message)
    } finally {
      setLoading(false)
    }
  }

  const disconnectLongbridge = async () => {
    try {
      const pending = liveRequestRef.current
      pending.controller?.abort()
      window.clearTimeout(pending.timer)
      liveRequestRef.current = { id: pending.id + 1, controller: null, timer: null }
      pendingLiveSymbolRef.current = null
      setLiveSwitch(null)
      const status = await apiJson('/api/connection', 'DELETE')
      setConnection(status)
      liveSequenceRef.current = -1
      setLiveFeed(null)
      setLiveSocketState('idle')
      if (mode === 'live') {
        setSession(null)
        setChain(null)
        setSurface(null)
      }
    } catch (reason) {
      setError(reason.message)
    }
  }

  const startLive = async () => {
    if (!connection.connected) {
      setCredentialOpen(true)
      return
    }
    const requestedSymbol = liveSymbolDraft.trim().toUpperCase().replace(/\.US$/, '')
    if (!/^[A-Z][A-Z0-9.-]{0,14}$/.test(requestedSymbol)) {
      setError('请输入有效的美股代码')
      return
    }
    const previous = liveRequestRef.current
    previous.controller?.abort()
    window.clearTimeout(previous.timer)
    const requestId = previous.id + 1
    const request = {
      symbol: requestedSymbol,
      expiration: liveFeed?.symbol === requestedSymbol && liveFeed.expirations.includes(expiration) ? expiration : null,
      pricing_mode: pricingMode,
      dealer_model: dealerModel,
      ...liveSettings,
    }
    pendingLiveSymbolRef.current = requestedSymbol
    setLiveSwitch({ status: 'switching', symbol: requestedSymbol, retryAt: null })

    const attempt = async () => {
      if (liveRequestRef.current.id !== requestId) return
      const controller = new AbortController()
      liveRequestRef.current = { id: requestId, controller, timer: null }
      setLoading(true)
      setError('')
      let retryScheduled = false
      try {
        const snapshot = await apiJson('/api/live/session', 'POST', request, { signal: controller.signal })
        if (liveRequestRef.current.id !== requestId) return
        pendingLiveSymbolRef.current = null
        setLiveSwitch(null)
        setLiveSymbolDraft(snapshot.feed.symbol)
        surfaceUpdateRef.current = { at: 0, key: '' }
        applyLiveSnapshot(snapshot, true)
      } catch (reason) {
        if (reason.name === 'AbortError' || liveRequestRef.current.id !== requestId) return
        if (reason.status === 429) {
          const retryAfterMs = Math.max(1000, Math.min(120000, Number(reason.retryAfterMs) || 65000))
          const retryAt = Date.now() + retryAfterMs
          retryScheduled = true
          setLiveSwitch({ status: 'queued', symbol: requestedSymbol, retryAt })
          setError(`${requestedSymbol} 已排队等待 Longbridge 限频窗口；当前 ${liveFeed?.symbol || activeSymbol} 行情继续推送，约 ${Math.ceil(retryAfterMs / 1000)} 秒后自动重试`)
          const timer = window.setTimeout(attempt, retryAfterMs)
          liveRequestRef.current = { id: requestId, controller: null, timer }
        } else {
          pendingLiveSymbolRef.current = null
          setLiveSwitch({ status: 'failed', symbol: requestedSymbol, retryAt: null })
          setError(reason.message)
        }
      } finally {
        if (liveRequestRef.current.id === requestId) {
          setLoading(false)
          if (!retryScheduled && liveRequestRef.current.controller === controller) {
            liveRequestRef.current = { id: requestId, controller: null, timer: null }
          }
        }
      }
    }

    liveRequestRef.current = { id: requestId, controller: null, timer: null }
    attempt()
  }

  useEffect(() => () => {
    liveRequestRef.current.controller?.abort()
    window.clearTimeout(liveRequestRef.current.timer)
  }, [])

  useEffect(() => {
    if (mode !== 'live' || !liveFeed || !connection.connected) return undefined
    let socket
    let retryTimer
    let stopped = false
    const connect = () => {
      if (stopped) return
      setLiveSocketState('connecting')
      socket = new WebSocket(websocketUrl('/api/live/stream'))
      socket.onopen = () => setLiveSocketState('streaming')
      socket.onmessage = (event) => {
        try {
          applyLiveSnapshot(JSON.parse(event.data))
        } catch {
          setError('实时行情消息无法解析')
        }
      }
      socket.onerror = () => socket.close()
      socket.onclose = () => {
        if (stopped) return
        setLiveSocketState('reconnecting')
        retryTimer = window.setTimeout(connect, 1500)
      }
    }
    connect()
    return () => {
      stopped = true
      window.clearTimeout(retryTimer)
      socket?.close()
    }
  }, [mode, Boolean(liveFeed), connection.connected, applyLiveSnapshot])


  useEffect(() => {
    if (mode !== 'replay' || !tradingDate || !symbols.length) return
    const controller = new AbortController()
    setLoading(true)
    setError('')
    api(`/api/session?symbols=${symbols.join(',')}&date=${tradingDate}`, controller.signal)
      .then((data) => {
        setSession(data)
        setFrame(Math.min(1, data.timeline.length - 1))
        setPlaying(false)
        const expirations = data.series[activeSymbol]?.expirations || []
        setExpiration(expirations.find((item) => (new Date(item) - new Date(tradingDate)) / 86400000 >= 7) || expirations[0] || '')
      })
      .catch((reason) => reason.name !== 'AbortError' && setError(reason.message))
      .finally(() => setLoading(false))
    return () => controller.abort()
  }, [mode, symbols.join(','), tradingDate])

  useEffect(() => {
    if (!session?.series[activeSymbol]) return
    const expirations = session.series[activeSymbol].expirations
    if (!expirations.includes(expiration)) setExpiration(expirations[0] || '')
  }, [activeSymbol, session])

  const minute = session?.timeline[frame] || ''
  useEffect(() => {
    if (mode !== 'replay' || !playing || !session) return undefined
    const timer = window.setInterval(() => {
      setFrame((current) => {
        if (current >= session.timeline.length - 1) {
          setPlaying(false)
          return current
        }
        return current + 1
      })
    }, Math.max(32, 1000 / speed))
    return () => window.clearInterval(timer)
  }, [mode, playing, speed, session])

  useEffect(() => {
    if (mode !== 'replay' || !minute || !expiration || !activeSymbol) return
    const controller = new AbortController()
    const timer = window.setTimeout(() => {
      api(`/api/chain?symbol=${activeSymbol}&date=${tradingDate}&minute=${minute}&expiration=${expiration}&pricing_mode=${pricingMode}&dealer_model=${dealerModel}`, controller.signal)
        .then(setChain)
        .catch((reason) => reason.name !== 'AbortError' && setError(reason.message))
    }, playing ? 100 : 0)
    return () => {
      window.clearTimeout(timer)
      controller.abort()
    }
  }, [mode, activeSymbol, tradingDate, minute, expiration, pricingMode, dealerModel])

  useEffect(() => {
    if (mode !== 'replay' || !minute) return
    const controller = new AbortController()
    api(`/api/surface?symbol=${activeSymbol}&date=${tradingDate}&minute=${minute}&max_dte=180`, controller.signal)
      .then(setSurface)
      .catch((reason) => reason.name !== 'AbortError' && setError(reason.message))
    return () => controller.abort()
  }, [mode, activeSymbol, tradingDate, expiration, Boolean(session), Math.floor(frame / 5)])

  useEffect(() => {
    if (mode !== 'replay' || !minute || !expiration) return
    const controller = new AbortController()
    api(`/api/volatility-context?symbol=${activeSymbol}&date=${tradingDate}&minute=${minute}&expiration=${expiration}`, controller.signal)
      .then(setVolContext)
      .catch((reason) => reason.name !== 'AbortError' && setError(reason.message))
    return () => controller.abort()
  }, [mode, activeSymbol, tradingDate, expiration])

  useEffect(() => {
    if (mode !== 'live' || !liveFeed || !connection.connected) return undefined
    let stopped = false
    const refresh = () => api('/api/live/volatility-context')
      .then((context) => !stopped && setVolContext(context))
      .catch((reason) => {
        if (!stopped) setError(reason.message)
      })
    refresh()
    const timer = window.setInterval(refresh, 60000)
    return () => {
      stopped = true
      window.clearInterval(timer)
    }
  }, [mode, activeSymbol, expiration, Boolean(liveFeed), connection.connected])

  useEffect(() => {
    setStrategyAnalysis(null)
  }, [mode, activeSymbol, expiration, pricingMode, dealerModel, strategyQuantity, strategyLegs])

  const currentBars = useMemo(() => {
    if (!session) return {}
    return Object.fromEntries(symbols.filter((symbol) => session.series[symbol]).map((symbol) => [symbol, session.series[symbol].bars.slice(0, frame + 1)]))
  }, [session, symbols, frame])

  const marketOption = useMemo(() => {
    if (!session) return null
    const times = session.timeline.slice(0, frame + 1)
    if (symbols.length > 1) {
      return {
        animation: false,
        tooltip: { trigger: 'axis', backgroundColor: '#111920', borderColor: '#34414d', textStyle: { color: '#dce5ec' } },
        legend: { top: 8, right: 12, textStyle: { color: '#8d9aa5' } },
        grid: { left: 54, right: 24, top: 42, bottom: 38 },
        xAxis: { type: 'category', data: times, boundaryGap: false, ...axis },
        yAxis: { type: 'value', scale: true, axisLabel: { formatter: '{value}%', color: '#83909c' }, ...axis },
        series: symbols.map((symbol, index) => {
          const bars = currentBars[symbol] || []
          const base = bars[0]?.close || 1
          return { name: symbol, type: 'line', showSymbol: false, smooth: false, lineStyle: { width: 1.6, color: PALETTE[index] }, data: bars.map((bar) => ((bar.close / base - 1) * 100).toFixed(3)) }
        }),
      }
    }
    const bars = currentBars[activeSymbol] || []
    return {
      animation: false,
      tooltip: { trigger: 'axis', axisPointer: { type: 'cross' }, backgroundColor: '#111920', borderColor: '#34414d', textStyle: { color: '#dce5ec' } },
      grid: [{ left: 54, right: 24, top: 26, height: '64%' }, { left: 54, right: 24, top: '76%', height: '15%' }],
      xAxis: [{ type: 'category', data: times, boundaryGap: true, ...axis }, { type: 'category', gridIndex: 1, data: times, axisLabel: { color: '#83909c', fontSize: 11 }, axisLine: axis.axisLine }],
      yAxis: [{ type: 'value', scale: true, ...axis }, { type: 'value', gridIndex: 1, splitNumber: 2, axisLabel: { show: false }, ...axis }],
      dataZoom: [{ type: 'inside', xAxisIndex: [0, 1], start: Math.max(0, 100 - 18000 / Math.max(bars.length, 1)), end: 100 }],
      series: [
        { name: activeSymbol, type: 'candlestick', data: bars.map((bar) => [bar.open, bar.close, bar.low, bar.high]), itemStyle: { color: '#37c99b', color0: '#ef6673', borderColor: '#37c99b', borderColor0: '#ef6673' }, markLine: focusStrike ? { silent: true, symbol: 'none', label: { formatter: `${focusStrike}`, color: '#f1c75b' }, lineStyle: { color: '#f1c75b', type: 'dashed' }, data: [{ yAxis: focusStrike }] } : undefined },
        { name: 'VWAP', type: 'line', showSymbol: false, data: bars.map((bar) => bar.vwap), lineStyle: { color: '#f1c75b', width: 1.2 }, smooth: false },
        { name: 'Volume', type: 'bar', xAxisIndex: 1, yAxisIndex: 1, data: bars.map((bar) => bar.volume), itemStyle: { color: '#334655' } },
      ],
    }
  }, [session, symbols, activeSymbol, currentBars, frame, focusStrike])

  const smileOption = useMemo(() => {
    if (!chain) return null
    const xValue = (row) => smileAxis === 'delta' ? row.delta : (smileAxis === 'moneyness' ? row.log_moneyness : row.strike)
    const make = (right, color) => ({
      name: right === 'CALL' ? 'Call IV' : 'Put IV', type: 'line', showSymbol: true, symbolSize: 5,
      data: chain.rows.filter((row) => row.right === right && row.moneyness >= 0.75 && row.moneyness <= 1.25 && row.quality_score >= 25).map((row) => [xValue(row), row.iv]),
      lineStyle: { color, width: 1.8 }, itemStyle: { color },
    })
    const fitted = smileAxis !== 'delta' && chain.svi ? [{ name: 'SVI', type: 'line', showSymbol: false, data: chain.svi.curve.map((row) => [smileAxis === 'strike' ? chain.forward * row.moneyness : Math.log(row.moneyness), row.iv]), lineStyle: { color: '#f1c75b', width: 2.1 } }] : []
    return { animation: false, tooltip: { trigger: 'axis', backgroundColor: '#111920', borderColor: '#34414d' }, legend: { top: 4, right: 8, textStyle: { color: '#8d9aa5' } }, grid: { left: 52, right: 18, top: 36, bottom: 34 }, xAxis: { type: 'value', name: smileAxis === 'delta' ? 'Delta' : smileAxis === 'moneyness' ? 'ln(K/F)' : 'Strike', nameTextStyle: { color: '#778590' }, scale: true, ...axis }, yAxis: { type: 'value', name: 'IV %', nameTextStyle: { color: '#778590' }, scale: true, ...axis }, series: [make('CALL', '#54d6b6'), make('PUT', '#ff7e8a'), ...fitted] }
  }, [chain, smileAxis])

  const residualOption = useMemo(() => chain?.svi ? ({
    animation: false, grid: { left: 45, right: 12, top: 18, bottom: 28 }, tooltip: { trigger: 'axis', backgroundColor: '#111920', borderColor: '#34414d' },
    xAxis: { type: 'value', name: 'ln(K/F)', scale: true, ...axis }, yAxis: { type: 'value', name: 'IV Δ', ...axis },
    series: [{ type: 'bar', data: chain.svi.residuals.map((row) => [row.k, row.residual]), itemStyle: { color: (params) => params.value[1] >= 0 ? '#37c99b' : '#ef6673' } }],
  }) : null, [chain])

  const gexOption = useMemo(() => chain ? ({
    animation: false, tooltip: { trigger: 'axis', backgroundColor: '#111920', borderColor: '#34414d' }, grid: { left: 68, right: 26, top: 24, bottom: 42 },
    xAxis: { type: 'category', data: chain.gex_by_strike.map((row) => row.strike), axisLabel: { interval: 'auto', color: '#83909c' }, ...axis },
    yAxis: { type: 'value', axisLabel: { formatter: (value) => formatCompact(value), color: '#83909c' }, ...axis },
    series: [{ type: 'bar', data: chain.gex_by_strike.map((row) => ({ value: row.gex, itemStyle: { color: row.gex >= 0 ? '#37c99b' : '#ef6673' } })) }],
  }) : null, [chain])

  const exposureOption = useMemo(() => {
    if (!chain?.quality?.gex_ready) return null
    const grouped = new Map()
    chain.rows.forEach((row) => {
      const value = grouped.get(row.strike) || { strike: row.strike, gex: 0, vanna: 0, charm: 0 }
      const sign = row.right === 'CALL' ? 1 : -1
      value.gex += row.gex || 0
      value.vanna += row.vanna * row.open_interest * 100 * sign
      value.charm += row.charm * row.open_interest * 100 * sign
      grouped.set(row.strike, value)
    })
    const values = [...grouped.values()].filter((row) => Math.abs(row.strike / chain.spot - 1) <= 0.12)
    return { animation: false, tooltip: { trigger: 'axis', backgroundColor: '#111920', borderColor: '#34414d' }, legend: { top: 2, right: 6, textStyle: { color: '#8d9aa5' } }, grid: { left: 55, right: 18, top: 32, bottom: 32 }, xAxis: { type: 'category', data: values.map((row) => row.strike), ...axis }, yAxis: { type: 'value', axisLabel: { formatter: formatCompact, color: '#83909c' }, ...axis }, series: [
      { name: 'GEX', type: 'bar', data: values.map((row) => row.gex), itemStyle: { color: '#54d6b6' } },
      { name: 'Vanna', type: 'line', showSymbol: false, data: values.map((row) => row.vanna), lineStyle: { color: '#70a5ff' } },
      { name: 'Charm', type: 'line', showSymbol: false, data: values.map((row) => row.charm), lineStyle: { color: '#ff7e8a' } },
    ] }
  }, [chain])

  const volOption = useMemo(() => volContext ? ({
    animation: false, grid: { left: 42, right: 12, top: 18, bottom: 28 }, tooltip: { trigger: 'axis', backgroundColor: '#111920', borderColor: '#34414d' },
    xAxis: { type: 'category', data: volContext.history.map((row) => row.date.slice(5)), ...axis }, yAxis: { type: 'value', scale: true, ...axis },
    series: [{ type: 'line', showSymbol: false, data: volContext.history.map((row) => row.iv), lineStyle: { color: '#b395ff', width: 1.8 }, areaStyle: { color: 'rgba(179,149,255,.08)' } }],
  }) : null, [volContext])

  const surfaceOption = useMemo(() => !surface ? null : !webgl ? ({
    animation: false,
    tooltip: { position: 'top', backgroundColor: '#111920', borderColor: '#34414d' },
    grid: { left: 58, right: 78, top: 20, bottom: 38 },
    xAxis: { type: 'category', name: 'Moneyness', data: surface.grid[0]?.map((cell) => cell[0].toFixed(3)) || [], ...axis },
    yAxis: { type: 'category', name: 'DTE', data: surface.grid.map((row) => `${row[0][1]}D`), ...axis },
    visualMap: { min: 10, max: 150, calculable: true, orient: 'vertical', right: 4, top: 20, textStyle: { color: '#8d9aa5' }, inRange: { color: ['#183c56', '#2b8f91', '#e3c65f', '#d95d6c'] } },
    series: [{ type: 'heatmap', data: surface.grid.flatMap((row, y) => row.map((cell, x) => [x, y, cell[2]])), emphasis: { itemStyle: { borderColor: '#dce5ec', borderWidth: 1 } } }],
  }) : ({
    animation: false, tooltip: {}, backgroundColor: 'transparent',
    visualMap: { show: true, min: 10, max: Math.min(150, Math.max(...surface.points.map((point) => point.iv), 80)), calculable: true, orient: 'horizontal', left: 20, bottom: 4, textStyle: { color: '#8d9aa5' }, inRange: { color: ['#183c56', '#2b8f91', '#e3c65f', '#d95d6c'] } },
    xAxis3D: { type: 'value', name: 'Moneyness', min: 0.75, max: 1.25, axisLabel: { color: '#83909c' } },
    yAxis3D: { type: 'value', name: 'DTE', axisLabel: { color: '#83909c' } },
    zAxis3D: { type: 'value', name: 'IV %', min: 0, max: 150, axisLabel: { color: '#83909c' } },
    grid3D: { boxWidth: 150, boxDepth: 90, environment: '#0d141a', axisLine: { lineStyle: { color: '#43515d' } }, splitLine: { lineStyle: { color: '#26313a' } }, viewControl: { distance: 190, alpha: 24, beta: 35 } },
    series: [
      {
        id: 'iv-surface',
        name: 'IV Surface',
        type: 'surface',
        shading: 'lambert',
        data: surface.grid.flat(),
        wireframe: { show: true, lineStyle: { color: 'rgba(205,224,232,.24)', width: 0.7 } },
        itemStyle: { opacity: 0.9 },
      },
      {
        id: 'observed-quotes',
        name: 'Observed',
        type: 'scatter3D',
        symbolSize: 2.2,
        data: surface.points.filter((point, index) => index % 4 === 0 && point.iv <= 150).map((point) => [point.moneyness, point.tte_days, point.iv]),
        itemStyle: { color: '#dce5ec', opacity: 0.34 },
      },
    ],
  }), [surface, webgl])

  const termOption = useMemo(() => surface ? ({
    animation: false,
    tooltip: { trigger: 'axis', backgroundColor: '#111920', borderColor: '#34414d' },
    grid: { left: 58, right: 26, top: 28, bottom: 42 },
    xAxis: { type: 'category', data: surface.term.map((point) => `${point.dte}D`), ...axis },
    yAxis: [{ type: 'value', name: 'ATM IV %', nameTextStyle: { color: '#778590' }, scale: true, ...axis }, { type: 'value', name: 'GEX', axisLabel: { formatter: formatCompact, color: '#83909c' }, splitLine: { show: false } }],
    series: [{ name: 'ATM IV', type: 'line', data: surface.term.map((point) => point.iv), showSymbol: true, symbolSize: 6, lineStyle: { color: '#70a5ff', width: 2 }, itemStyle: { color: '#70a5ff' }, areaStyle: { color: 'rgba(112,165,255,.10)' } }, { name: 'Expiry GEX', type: 'bar', yAxisIndex: 1, data: surface.term.map((point) => point.net_gex), itemStyle: { color: 'rgba(84,214,182,.3)' } }],
  }) : null, [surface])

  const liveStrategyLegs = useMemo(() => strategyLegs.map((leg) => {
    const current = chain?.rows.find((row) => row.strike === leg.strike && row.right === leg.right)
    return { ...leg, ...(current || {}) }
  }), [strategyLegs, chain])

  const payoffOption = useMemo(() => {
    const payoff = strategyAnalysis?.payoff
    if (!payoff?.length) return null
    return { animation: false, grid: { left: 52, right: 14, top: 18, bottom: 30 }, tooltip: { trigger: 'axis', backgroundColor: '#111920', borderColor: '#34414d' }, xAxis: { type: 'category', data: payoff.map((row) => row[0]), ...axis }, yAxis: { type: 'value', axisLabel: { formatter: formatCompact, color: '#83909c' }, ...axis }, series: [{ type: 'line', showSymbol: false, data: payoff.map((row) => row[1]), lineStyle: { color: '#f1c75b', width: 2 }, areaStyle: { color: 'rgba(241,199,91,.08)' }, markLine: { symbol: 'none', data: [{ yAxis: 0 }], lineStyle: { color: '#56636e' } } }] }
  }, [strategyAnalysis])

  const compareMetrics = useMemo(() => {
    if (!chain || !compareChain) return null
    return {
      spot: chain.spot - compareChain.spot,
      atmIv: (chain.metrics.atm_iv || 0) - (compareChain.metrics.atm_iv || 0),
      rr25: (chain.metrics.rr25 || 0) - (compareChain.metrics.rr25 || 0),
      netGex: chain.metrics.net_gex == null || compareChain.metrics.net_gex == null ? null : chain.metrics.net_gex - compareChain.metrics.net_gex,
    }
  }, [chain, compareChain])

  const strategyRequest = () => ({
    mode,
    symbol: activeSymbol,
    date: mode === 'replay' ? tradingDate : null,
    minute: mode === 'replay' ? minute : null,
    expiration,
    pricing_mode: pricingMode,
    dealer_model: dealerModel,
    quantity: strategyQuantity,
    legs: liveStrategyLegs.map((leg) => ({
      symbol: leg.symbol || null,
      strike: leg.strike,
      right: leg.right,
      side: leg.side,
      ratio: leg.ratio,
    })),
  })

  const analyzeCurrentStrategy = async () => {
    if (!chain || !liveStrategyLegs.length) return
    setAnalyzing(true)
    setError('')
    try {
      setStrategyAnalysis(await apiJson('/api/strategy/analyze', 'POST', strategyRequest()))
    } catch (reason) {
      setError(reason.message)
    } finally {
      setAnalyzing(false)
    }
  }

  const captureAudit = async (kind = 'research_snapshot') => {
    if (!chain) return
    try {
      await apiJson('/api/audit/records', 'POST', {
        kind,
        mode,
        symbol: activeSymbol,
        snapshot_id: chain.snapshot_id,
        payload: { chain, surface, volatility: volContext, strategy: strategyAnalysis },
      })
      await refreshAudit()
    } catch (reason) {
      setError(reason.message)
    }
  }

  const selectAudit = async (summary) => {
    try {
      const record = await api(`/api/audit/records/${summary.id}`)
      setCompareBookmark(record)
      setCompareChain(record.payload?.chain || null)
    } catch (reason) {
      setError(reason.message)
    }
  }

  const submitPaperStrategy = async () => {
    if (!strategyAnalysis || paperConfirmation !== 'PAPER') return
    setLoading(true)
    setError('')
    try {
      await apiJson('/api/trade/orders', 'POST', {
        preview_id: strategyAnalysis.preview_id,
        confirmation: paperConfirmation,
        strategy: strategyRequest(),
      })
      setPaperConfirmOpen(false)
      setPaperConfirmation('')
      await refreshTrading()
      await refreshAudit()
    } catch (reason) {
      setError(reason.message)
    } finally {
      setLoading(false)
    }
  }

  const cancelPaperOrder = async (orderId) => {
    try {
      await apiJson(`/api/trade/orders/${encodeURIComponent(orderId)}`, 'DELETE')
      await refreshTrading()
    } catch (reason) {
      setError(reason.message)
    }
  }

  const exportSnapshot = () => {
    if (!chain || !minute) return
    const payload = { chain, surface, volatility: volContext, strategy: strategyAnalysis, compare: compareMetrics }
    const url = URL.createObjectURL(new Blob([JSON.stringify(payload, null, 2)], { type: 'application/json' }))
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `${activeSymbol}-${tradingDate}-${minute.replace(':', '')}-snapshot.json`
    anchor.click()
    URL.revokeObjectURL(url)
  }
  const addStrategyLeg = (row) => {
    setStrategyLegs((current) => current.some((leg) => leg.strike === row.strike && leg.right === row.right) ? current : [...current, { symbol: row.symbol, strike: row.strike, right: row.right, side: 'BUY', ratio: 1 }])
    setFocusStrike(row.strike)
  }
  const applyPreset = (preset) => {
    if (!chain?.rows?.length) return
    const find = (right, target) => chain.rows.filter((row) => row.right === right).sort((a, b) => Math.abs(a.strike - target) - Math.abs(b.strike - target))[0]
    const spot = chain.spot
    const leg = (row, side) => ({ symbol: row.symbol, strike: row.strike, right: row.right, side, ratio: 1 })
    if (preset === 'straddle') setStrategyLegs([leg(find('CALL', spot), 'BUY'), leg(find('PUT', spot), 'BUY')])
    if (preset === 'strangle') setStrategyLegs([leg(find('CALL', spot * 1.03), 'BUY'), leg(find('PUT', spot * 0.97), 'BUY')])
    if (preset === 'bull_call') setStrategyLegs([leg(find('CALL', spot), 'BUY'), leg(find('CALL', spot * 1.03), 'SELL')])
    if (preset === 'iron_condor') setStrategyLegs([leg(find('PUT', spot * 0.94), 'BUY'), leg(find('PUT', spot * 0.97), 'SELL'), leg(find('CALL', spot * 1.03), 'SELL'), leg(find('CALL', spot * 1.06), 'BUY')])
  }

  const activeBar = session?.series[activeSymbol]?.bars[frame]
  const quotePermission = connection.quote_level?.includes('USO') ? 'US Options LV1' : (connection.quote_level ? 'OpenAPI Quotes' : '等待凭证')
  const chainQualityReady = Boolean(chain?.quality?.gex_ready)
    && (chain?.quality?.fresh_quote_coverage_pct ?? 0) >= 80
    && (chain?.quality?.spot_age_ms ?? 0) <= 5000
  const chainQualityLabel = !chain ? '等待截面' : (chain?.quality?.spot_age_ms ?? 0) > 5000
    ? '现货报价陈旧'
    : (chain?.quality?.fresh_quote_coverage_pct ?? 0) < 80
      ? '期权报价陈旧'
      : chain?.quality?.gex_ready ? '完整截面' : '元数据受限'
  const chartViewKey = `${mode}:${activeSymbol}:${expiration || 'none'}`
  const marketViewKey = `${mode}:${activeSymbol}`
  const sviLabel = chain?.svi
    ? 'SVI ready'
    : chain?.svi_diagnostics
      ? `SVI ${chain.svi_diagnostics.eligible_samples}/${chain.svi_diagnostics.required_samples}`
      : 'SVI --'
  const addSymbol = (symbol) => {
    if (!symbol) return
    if (mode === 'live') {
      setLiveSymbolDraft(symbol)
      return
    }
    if (!symbols.includes(symbol) && symbols.length < 5) setSymbols([...symbols, symbol])
  }
  const removeSymbol = (symbol) => {
    if (mode === 'live' || symbols.length === 1) return
    const next = symbols.filter((item) => item !== symbol)
    setSymbols(next)
    if (activeSymbol === symbol) setActiveSymbol(next[0])
  }

  return (
    <div className="app-shell">
      <header className="topbar">
        <div className="brand"><Activity size={20} /><strong>Option Workstation</strong><span>Rust Analytics</span></div>
        <div className="session-controls">
          <div className="segments mode-switch" aria-label="数据模式">
            <button className={mode === 'replay' ? 'active' : ''} onClick={() => switchMode('replay')} title="历史回放"><History size={13} />历史</button>
            <button className={mode === 'live' ? 'active' : ''} onClick={() => switchMode('live')} title="实时工作台"><Radio size={13} />实时</button>
          </div>
          {mode === 'replay' ? <>
            <div className="symbol-strip">
              {symbols.map((symbol, index) => <button key={symbol} className={`symbol-chip ${activeSymbol === symbol ? 'active' : ''}`} style={{ '--accent': PALETTE[index] }} onClick={() => setActiveSymbol(symbol)}>{symbol}{symbols.length > 1 && <X size={13} onClick={(event) => { event.stopPropagation(); removeSymbol(symbol) }} />}</button>)}
              <label className="icon-button" title="添加标的"><Plus size={16} /><select value="" onChange={(event) => addSymbol(event.target.value)}><option value="">添加</option>{catalog?.symbols.filter((symbol) => !symbols.includes(symbol)).map((symbol) => <option key={symbol}>{symbol}</option>)}</select></label>
            </div>
            <select className="date-select" value={tradingDate} onChange={(event) => setTradingDate(event.target.value)}>{catalog?.common_dates.map((item) => <option key={item}>{item}</option>)}</select>
          </> : <label className="live-symbol-control" title="实时美股代码"><span>US</span><input list="live-symbols" value={liveSymbolDraft} maxLength={15} onChange={(event) => setLiveSymbolDraft(event.target.value.toUpperCase())} onKeyDown={(event) => event.key === 'Enter' && startLive()} aria-label="实时美股代码" /><datalist id="live-symbols">{catalog?.symbols.map((symbol) => <option key={symbol} value={symbol} />)}</datalist></label>}
          <select className="date-select compact" value={pricingMode} onChange={(event) => setPricingMode(event.target.value)}><option value="micro">Micro</option><option value="mid">Mid</option><option value="ask">Ask</option></select>
          <select className="date-select compact" value={dealerModel} onChange={(event) => setDealerModel(event.target.value)}><option value="classic">Call+/Put-</option><option value="short_all">Dealer Short</option><option value="long_all">Dealer Long</option></select>
          <div className="segments layout-switch" aria-label="工作台布局">{[['dense', '总览'], ['vol', '波动率'], ['trade', '交易']].map(([value, label]) => <button key={value} className={layout === value ? 'active' : ''} onClick={() => setLayout(value)}>{label}</button>)}</div>
          <a className="icon-button action" href="/guide.html" title="打开初学者指南" aria-label="打开初学者指南"><BookOpen size={15} /></a>
          <button className="icon-button action" title="写入审计账本" onClick={() => captureAudit()}><Bookmark size={15} /></button>
          <button className="icon-button action" title="导出研究快照" onClick={exportSnapshot}><Download size={15} /></button>
          <button className={`connection-button ${connection.connected ? 'connected' : ''}`} onClick={() => setCredentialOpen(true)} title="Longbridge 连接设置">{connection.connected ? <Wifi size={14} /> : <WifiOff size={14} />}<span>{connection.connected ? connection.account_hint || '已连接' : 'Longbridge'}</span></button>
          {mode === 'live' && <button className="live-run" onClick={startLive} disabled={loading} title="启动或更新实时订阅"><Radio size={14} />{liveSwitch?.status === 'queued' ? `${liveSwitch.symbol} 排队中` : liveFeed ? '更新订阅' : '启动实时'}</button>}
        </div>
      </header>

      <main className={`research-workspace layout-${layout}`}>
        <section className="workspace-panel market-panel">
          <div className="market-heading"><div><span className="eyebrow">{activeSymbol} · {mode === 'live' ? 'LONGBRIDGE LIVE' : tradingDate}</span><h1>{activeBar ? activeBar.close.toFixed(2) : '--'} <small>{minute || '--:--'} ET</small></h1></div><div className="ohlc"><span>O <b>{activeBar?.open.toFixed(2)}</b></span><span>H <b>{activeBar?.high.toFixed(2)}</b></span><span>L <b>{activeBar?.low.toFixed(2)}</b></span><span>V <b>{formatCompact(activeBar?.volume)}</b></span></div></div>
          <Chart option={marketOption} className="market-chart" viewKey={marketViewKey} />
        </section>

        <aside className="workspace-panel snapshot-panel">
          <div className="panel-title"><span>期权截面 <small>{chain?.provenance?.source || '--'}</small></span><select value={expiration} onChange={(event) => setExpiration(event.target.value)}>{session?.series[activeSymbol]?.expirations.map((item) => <option key={item} value={item}>{item} · {Math.max(0, Math.round((new Date(`${item}T16:00:00`) - new Date(`${tradingDate}T09:30:00`)) / 86400000))}D</option>)}</select></div>
          <div className={`quality-banner ${chainQualityReady ? 'ready' : 'limited'}`}><span>{chainQualityLabel}</span><b>Q {chain?.quality?.quote_coverage_pct?.toFixed(0) ?? '--'} · Fresh {chain?.quality?.fresh_quote_coverage_pct?.toFixed(0) ?? '--'} · Meta {chain?.quality?.metadata_coverage_pct?.toFixed(0) ?? '--'}%</b></div>
          <div className="metric-grid">
            <Metric label="Spot" value={chain?.spot?.toFixed(2)} />
            <Metric label="ATM IV" value={chain?.metrics?.atm_iv ? `${chain.metrics.atm_iv.toFixed(1)}%` : '--'} />
            <Metric label="25Δ RR" value={chain?.metrics?.rr25?.toFixed(2)} tone={chain?.metrics?.rr25 >= 0 ? 'up' : 'down'} />
            <Metric label="25Δ BF" value={chain?.metrics?.bf25?.toFixed(2)} />
            <Metric label="Net GEX" value={formatCompact(chain?.metrics?.net_gex)} tone={chain?.metrics?.net_gex == null ? '' : chain.metrics.net_gex >= 0 ? 'up' : 'down'} detail={chain?.quality?.gex_ready ? '' : chain?.quality?.blocked_metrics?.join(' · ')} />
            <Metric label="Gamma Flip" value={chain?.metrics?.gamma_flip?.toFixed(2)} />
            <Metric label="Quality" value={chain?.metrics?.avg_quality?.toFixed(0)} />
            <Metric label="PCR" value={chain?.metrics?.pcr?.toFixed(2)} />
          </div>
          <div className="oi-balance"><div><span>Call OI</span><b>{formatCompact(chain?.metrics?.call_oi)}</b></div><div><span>Put OI</span><b>{formatCompact(chain?.metrics?.put_oi)}</b></div><div className="balance-track"><i style={{ width: `${Math.min(100, (chain?.metrics?.call_oi || 0) / Math.max(1, (chain?.metrics?.call_oi || 0) + (chain?.metrics?.put_oi || 0)) * 100)}%` }} /></div></div>
        </aside>

        <Panel id="volatility" className="vol-panel" title="波动率状态" icon={<Gauge size={14} />} tools={<span className={volContext?.status === 'ready' ? 'ok-text' : 'warning-text'}>{volContext?.status || 'loading'}</span>}>
          <div className="compact-metrics"><Metric label="IV Rank" value={volContext?.iv_rank != null ? `${volContext.iv_rank.toFixed(1)}%` : '--'} detail={`Matched DTE samples: ${volContext?.sample_size ?? 0}`} /><Metric label="IV Percentile" value={volContext?.iv_percentile != null ? `${volContext.iv_percentile.toFixed(1)}%` : '--'} detail={volContext?.history_through ? `History through ${volContext.history_through}` : 'No matched-DTE history'} /><Metric label="RV20" value={volContext?.realized_volatility?.['20'] != null ? `${volContext.realized_volatility['20'].toFixed(1)}%` : '--'} detail={volContext?.rv_through ? `Close through ${volContext.rv_through}` : 'Daily close history unavailable'} /><Metric label="VRP20" value={volContext?.vrp20?.toFixed(2)} /><Metric label="Expected Move" value={volContext?.expected_move?.toFixed(2)} detail={volContext?.expected_move_basis || ''} /><Metric label="Snapshot" value={chain?.snapshot_id?.slice(0, 8)} /></div>
          <div className="vol-provenance"><span>{volContext?.iv_source || '--'}</span><span>{volContext?.rv_source || '--'}</span><span>{volContext?.sample_size ?? 0} samples</span></div>
          <Chart option={volOption} viewKey={chartViewKey} />
        </Panel>

        <Panel id="smile" className="smile-panel" title="IV 微笑与 SVI" icon={<Gauge size={14} />} tools={<div className="svi-toolbar"><span className={chain?.svi ? 'ok-text' : 'warning-text'}>{sviLabel}</span><div className="segments mini">{['strike', 'moneyness', 'delta'].map((item) => <button key={item} className={smileAxis === item ? 'active' : ''} onClick={() => setSmileAxis(item)}>{item === 'moneyness' ? 'ln(K/F)' : item}</button>)}</div></div>}>
          <Chart option={smileOption} viewKey={chartViewKey} />
        </Panel>
        <Panel id="residual" className="residual-panel" title="SVI 残差 / 约束检查" icon={<Activity size={14} />} tools={<span className={chain?.svi?.butterfly_violations ? 'warning-text' : 'ok-text'}>BFLY {chain?.svi?.butterfly_violations ?? '--'}</span>}>
          {residualOption ? <Chart option={residualOption} viewKey={chartViewKey} /> : <div className="empty-state">{chain?.svi_diagnostics?.reason || '等待足够的 OTM 报价后拟合 SVI'}</div>}
        </Panel>
        <Panel id="term" className="term-panel" title="期限结构" icon={<Activity size={14} />} tools={<span className={surface?.arbitrage?.calendar_violations ? 'warning-text' : 'ok-text'}>CAL {surface?.arbitrage?.calendar_violations ?? '--'}</span>}>
          <Chart option={termOption} viewKey={chartViewKey} />
        </Panel>
        <Panel id="exposure" className="exposure-panel" title="Dealer Exposure" icon={<BarChart3 size={14} />} tools={<span className={chain?.quality?.gex_ready ? 'ok-text' : 'warning-text'}>{chain?.quality?.gex_ready ? 'OI ready' : 'OI blocked'}</span>}>
          {exposureOption ? <Chart option={exposureOption} viewKey={chartViewKey} onEvents={{ click: (params) => params.name && setFocusStrike(Number(params.name)) }} /> : <div className="empty-state">OI 元数据覆盖不足，GEX / Vanna / Charm 暂停计算</div>}
        </Panel>

        <Panel id="surface" className="surface-panel" title="约束 IV 曲面" icon={<Layers3 size={14} />} tools={<span className={surface?.arbitrage?.trusted ? 'ok-text' : 'warning-text'}>{surface?.arbitrage?.trusted ? 'Trusted' : 'Research'} {surface?.arbitrage?.confidence_score?.toFixed(0) ?? '--'} · C{surface?.arbitrage?.price_convexity_violations ?? '--'} · M{surface?.arbitrage?.price_monotonicity_violations ?? '--'}</span>}>
          {surface?.arbitrage?.warning && <div className="surface-warning">{surface.arbitrage.warning}</div>}
          {webgl ? <Suspense fallback={<div className="empty-state">加载 3D 渲染器…</div>}><SurfaceChart option={surfaceOption} viewKey={chartViewKey} /></Suspense> : <Chart option={surfaceOption} incremental viewKey={chartViewKey} />}
        </Panel>
        <Panel id="strategy" className="strategy-panel" title="策略风险引擎" icon={<TableProperties size={14} />} tools={<button className="text-button" onClick={() => setStrategyLegs([])}>清空</button>}>
          <StrategyWorkbench legs={liveStrategyLegs} setLegs={setStrategyLegs} analysis={strategyAnalysis} quantity={strategyQuantity} setQuantity={setStrategyQuantity} onAnalyze={analyzeCurrentStrategy} payoffOption={payoffOption} onPreset={applyPreset} tradeAccount={tradeAccount} onPaperSubmit={() => setPaperConfirmOpen(true)} analyzing={analyzing} paperEligible={mode === 'live'} />
        </Panel>

        <Panel id="audit" className="audit-panel" title="研究审计账本" icon={<Bookmark size={14} />}>
          <AuditPanel records={auditRecords} selectedId={compareBookmark?.id} onSelect={selectAudit} onCapture={() => captureAudit()} />
          {compareMetrics && <div className="compare-grid"><Metric label="Δ Spot" value={compareMetrics.spot.toFixed(2)} /><Metric label="Δ ATM IV" value={compareMetrics.atmIv.toFixed(2)} /><Metric label="Δ RR25" value={compareMetrics.rr25.toFixed(2)} /><Metric label="Δ Net GEX" value={formatCompact(compareMetrics.netGex)} /></div>}
        </Panel>
        <Panel id="execution" className="execution-panel" title="Paper 执行监控" icon={<LockKeyhole size={14} />} tools={<span className={tradeAccount?.execution_enabled ? 'ok-text' : 'warning-text'}>{tradeAccount?.execution_enabled ? 'Enabled' : 'Server locked'}</span>}>
          <ExecutionPanel account={tradeAccount} orders={orders} onRefresh={refreshTrading} onCancel={cancelPaperOrder} />
        </Panel>
        <Panel id="chain" className="chain-panel" title="镜像期权链" icon={<TableProperties size={14} />} tools={<span className="muted-label">Q {chain?.quality?.usable_pct ?? '--'}% · {pricingMode}</span>}>
          <ChainTable chain={chain} onAdd={addStrategyLeg} onFocus={setFocusStrike} focusStrike={focusStrike} />
        </Panel>
      </main>

      {mode === 'replay' ? <footer className="playback-dock">
        <div className="transport">
          <button title="回到开盘" onClick={() => { setFrame(0); setPlaying(false) }}><RotateCcw size={17} /></button>
          <button title="上一帧" onClick={() => { setFrame(Math.max(0, frame - 1)); setPlaying(false) }}><ChevronLeft size={19} /></button>
          <button className="play" title={playing ? '暂停' : '播放'} onClick={() => setPlaying(!playing)}>{playing ? <Pause size={19} /> : <Play size={19} />}</button>
          <button title="下一帧" onClick={() => { setFrame(Math.min((session?.timeline.length || 1) - 1, frame + 1)); setPlaying(false) }}><ChevronRight size={19} /></button>
        </div>
        <div className="timeline"><input type="range" min="0" max={Math.max(0, (session?.timeline.length || 1) - 1)} value={frame} onChange={(event) => { setFrame(Number(event.target.value)); setPlaying(false) }} /><div className="timeline-labels"><span>09:30</span><strong>{minute || '--:--'} ET</strong><span>16:00</span></div></div>
        <div className="speed-control"><span>速度</span><div className="segments">{SPEEDS.map((item) => <button key={item} className={speed === item ? 'active' : ''} onClick={() => setSpeed(item)}>{item}×</button>)}</div></div>
      </footer> : <footer className="live-dock">
        <div className={`feed-health ${liveSocketState === 'streaming' ? 'healthy' : ''}`}>
          {liveSocketState === 'streaming' ? <Wifi size={16} /> : <WifiOff size={16} />}
          <div><span>行情推送</span><strong>{connection.connected ? liveSocketState : '未连接'}</strong></div>
        </div>
        <div className="live-readouts">
          <LiveReadout label="数据时刻" value={liveFeed?.as_of ? new Date(liveFeed.as_of).toLocaleTimeString('zh-CN', { hour12: false }) : '--:--:--'} />
          <LiveReadout label="订阅合约" value={`${liveFeed?.subscribed_contracts || connection.subscribed_contracts || 0} / 500`} />
          <LiveReadout label="Fresh Quote" value={`${liveFeed?.fresh_quote_coverage_pct?.toFixed(0) ?? 0}%`} tone={liveFeed?.fresh_quote_coverage_pct >= 90 ? 'healthy' : 'warning'} />
          <LiveReadout label="OI 覆盖" value={`${liveFeed?.metadata_coverage_pct?.toFixed(0) ?? 0}%`} tone={liveFeed?.metadata_coverage_pct >= 90 ? 'healthy' : 'warning'} />
          <LiveReadout label="传输延迟" value={liveFeed?.latency_ms == null ? '--' : `${liveFeed.latency_ms} ms`} tone={liveFeed?.latency_ms < 3000 ? 'healthy' : 'warning'} />
          <LiveReadout label="质量状态" value={liveFeed?.quality_state || '--'} tone={liveFeed?.quality_state === 'ready' ? 'healthy' : 'warning'} />
        </div>
        <div className="live-settings">
          <label title="期权订阅上限"><span>合约</span><input type="number" min="20" max="480" step="20" value={liveSettings.max_contracts} onChange={(event) => setLiveSettings((current) => ({ ...current, max_contracts: Number(event.target.value) }))} /></label>
          <label title="用于 IV 曲面的到期日数量"><span>期限</span><input type="number" min="2" max="6" value={liveSettings.surface_expiries} onChange={(event) => setLiveSettings((current) => ({ ...current, surface_expiries: Number(event.target.value) }))} /></label>
          <label title="现价上下的筛选窗口"><span>价宽 %</span><input type="number" min="4" max="30" step="1" value={Math.round(liveSettings.moneyness_window * 100)} onChange={(event) => setLiveSettings((current) => ({ ...current, moneyness_window: Number(event.target.value) / 100 }))} /></label>
          <button onClick={startLive} disabled={loading}><RefreshCw size={14} />应用</button>
        </div>
      </footer>}

      {credentialOpen && <div className="credential-backdrop" onMouseDown={() => setCredentialOpen(false)}>
        <aside className="credential-drawer" role="dialog" aria-modal="true" aria-labelledby="credential-title" onMouseDown={(event) => event.stopPropagation()}>
          <div className="credential-header"><div><KeyRound size={18} /><div><strong id="credential-title">Longbridge OpenAPI</strong><span>实时行情连接</span></div></div><button className="drawer-close" title="关闭" onClick={() => setCredentialOpen(false)}><X size={17} /></button></div>
          <div className={`connection-summary ${connection.connected ? 'connected' : ''}`}>
            {connection.connected ? <Wifi size={17} /> : <WifiOff size={17} />}
            <div><strong>{connection.connected ? `已连接 ${connection.account_hint || ''}` : '尚未连接'}</strong><span>{quotePermission} · {connection.state}</span></div>
          </div>
          {connection.connected && <div className="trade-connection-grid"><div><span>Trade API</span><b>{connection.trade_connected ? 'Connected' : 'Unavailable'}</b></div><div><span>Account</span><b>{connection.account_type || '--'}</b></div><div><span>Buying Power</span><b>{connection.buy_power || '--'}</b></div><div><span>Paper Orders</span><b className={connection.order_execution_enabled ? 'ok-text' : 'warning-text'}>{connection.order_execution_enabled ? 'Enabled' : 'Locked'}</b></div></div>}
          <form className="credential-form" onSubmit={submitCredentials} autoComplete="off">
            <label htmlFor="lb-app-key"><span>App Key</span><input id="lb-app-key" type="password" autoComplete="new-password" value={credentials.app_key} onChange={(event) => setCredentials((current) => ({ ...current, app_key: event.target.value }))} /></label>
            <label htmlFor="lb-app-secret"><span>App Secret</span><input id="lb-app-secret" type="password" autoComplete="new-password" value={credentials.app_secret} onChange={(event) => setCredentials((current) => ({ ...current, app_secret: event.target.value }))} /></label>
            <label htmlFor="lb-access-token"><span>Access Token</span><textarea id="lb-access-token" rows="4" autoComplete="off" value={credentials.access_token} onChange={(event) => setCredentials((current) => ({ ...current, access_token: event.target.value }))} /></label>
            <div className="credential-security"><LockKeyhole size={15} /><span>凭证通过同源接口传给 Rust 后端，仅驻留当前进程内存；刷新页面不会回填，服务重启后自动清除。</span></div>
            {connection.error && <div className="connection-error">{connection.error}</div>}
            <button className="credential-submit" type="submit" disabled={loading}>{loading ? <RefreshCw className="spin" size={15} /> : <KeyRound size={15} />}{connection.connected ? '更换并验证凭证' : '验证并连接'}</button>
          </form>
          <div className="package-section"><span>行情权限包</span><div>{connection.packages?.length ? connection.packages.map((item) => <b key={item}>{item}</b>) : <small>连接后读取；美股期权实时行情需要相应 OPRA 权限。</small>}</div></div>
          {connection.connected && <button className="disconnect-button" onClick={disconnectLongbridge}><WifiOff size={14} />断开并清除凭证</button>}
        </aside>
      </div>}
      {paperConfirmOpen && <div className="paper-confirm-backdrop" onMouseDown={() => setPaperConfirmOpen(false)}>
        <div className="paper-confirm-dialog" role="dialog" aria-modal="true" aria-labelledby="paper-confirm-title" onMouseDown={(event) => event.stopPropagation()}>
          <div className="paper-confirm-header"><div><LockKeyhole size={17} /><strong id="paper-confirm-title">确认纸面组合订单</strong></div><button title="关闭" onClick={() => setPaperConfirmOpen(false)}><X size={16} /></button></div>
          <p>将按当前可执行买卖价重新校验预览，并以限价逐腿提交到已识别的模拟账户。买腿优先；后续腿失败时会请求撤销先前订单。</p>
          <label><span>输入 PAPER 继续</span><input autoFocus value={paperConfirmation} onChange={(event) => setPaperConfirmation(event.target.value.toUpperCase())} /></label>
          <button className="paper-confirm-submit" disabled={paperConfirmation !== 'PAPER' || !tradeAccount?.execution_enabled} onClick={submitPaperStrategy}>提交模拟订单</button>
        </div>
      </div>}
      {(loading || error) && <div className={`status-toast ${error ? 'error' : ''}`}>{loading ? <><RefreshCw className="spin" size={15} />{mode === 'live' ? '建立实时行情会话' : '读取历史分区'}</> : error}</div>}
    </div>
  )
}


export default App
