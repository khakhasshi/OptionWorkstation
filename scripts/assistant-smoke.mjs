#!/usr/bin/env node

const baseUrl = process.argv[2] || 'http://127.0.0.1:7311'
const requested = (process.env.ASSISTANT_SMOKE_SYMBOLS || 'AAPL,MSFT,GOOGL,AMZN,NVDA,META,TSLA,QQQ,SPY')
  .split(',')
  .map((value) => value.trim().toUpperCase())
  .filter(Boolean)

async function json(path, options) {
  const response = await fetch(`${baseUrl}${path}`, options)
  const payload = await response.json().catch(() => ({}))
  if (!response.ok) throw new Error(`${response.status} ${path}: ${payload.detail || response.statusText}`)
  return payload
}

function replayReference(symbol, date, session) {
  const minute = session.timeline.includes('10:00')
    ? '10:00'
    : session.timeline[Math.min(30, session.timeline.length - 1)]
  const expirations = session.series[symbol].expirations
  if (!minute || !expirations.length) throw new Error(`${symbol} ${date}: replay session is incomplete`)
  return {
    kind: 'replay',
    symbol,
    date,
    minute,
    expiration: expirations[0],
    pricing_mode: 'micro',
    dealer_model: 'classic',
    max_dte: 180,
  }
}

async function streamMessage(sessionId, contextRefs, message) {
  const response = await fetch(`${baseUrl}/api/assistant/sessions/${sessionId}/messages`, {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ message, context_refs: contextRefs, strategy: null }),
  })
  const body = await response.text()
  if (!response.ok) throw new Error(`${response.status} assistant stream: ${body}`)
  if (!body.includes('event: meta') || !body.includes('event: done')) {
    throw new Error('assistant stream did not contain meta and done events')
  }
  if (body.includes('\uFFFD')) throw new Error('assistant stream contains invalid Unicode replacement characters')
  return body
}

const status = await json('/api/assistant/status')
if (!status.enabled || status.provider !== 'mock') {
  throw new Error('start the server with OPTION_WORKSTATION_LLM_MOCK=1')
}

const catalog = await json('/api/catalog')
const symbols = requested.filter((symbol) => catalog.symbols.includes(symbol))
if (!symbols.length) throw new Error('none of the requested symbols are present in the data root')

const references = []
for (const symbol of symbols) {
  const dates = catalog.dates_by_symbol[symbol] || []
  const date = dates.at(-1)
  if (!date) throw new Error(`${symbol}: no replay dates`)
  const replay = await json(`/api/session?symbols=${encodeURIComponent(symbol)}&date=${date}`)
  const reference = replayReference(symbol, date, replay)
  const created = await json('/api/assistant/sessions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ title: `${symbol} smoke` }),
  })
  await streamMessage(created.id, [reference], `验证 ${symbol} 冻结截面`)
  const stored = await json(`/api/assistant/sessions/${created.id}`)
  if (stored.messages.length !== 2 || stored.contexts.length !== 1) {
    throw new Error(`${symbol}: assistant session was not persisted in memory`)
  }
  references.push(reference)
  process.stdout.write(`assistant context ok ${symbol} ${date} ${reference.minute}\n`)
}

if (references.length >= 2) {
  const created = await json('/api/assistant/sessions', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ title: 'two snapshot comparison' }),
  })
  await streamMessage(created.id, references.slice(0, 2), '比较两个截面的口径与风险状态')
  const stored = await json(`/api/assistant/sessions/${created.id}`)
  if (stored.contexts.length !== 2) throw new Error('two-snapshot comparison did not retain both contexts')
  const favorite = await json(`/api/assistant/sessions/${created.id}/favorite`, {
    method: 'POST',
  })
  if (favorite.kind !== 'assistant_analysis') throw new Error('assistant favorite was not audited')
  const imported = await json('/api/assistant/sessions/import', {
    method: 'POST',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify({ audit_record_id: favorite.id }),
  })
  if (imported.imported_from !== favorite.id || imported.contexts.length !== 2) {
    throw new Error('assistant favorite import did not restore the frozen contexts')
  }
}

process.stdout.write(`assistant smoke passed: ${symbols.length} symbols plus two-context comparison\n`)
