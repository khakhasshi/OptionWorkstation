import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import {
  ArrowDown,
  Bookmark,
  Bot,
  Check,
  History,
  Link2,
  MessageSquarePlus,
  Plus,
  Send,
  Trash2,
  X,
} from 'lucide-react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { api, apiEventStream, apiJson } from '../lib/api'

const ACTIVE_SESSION_KEY = 'option-workstation-assistant-session-v1'
const QUICK_PROMPTS = [
  ['解读截面', '请按数据质量、波动率定价、Dealer Exposure、可执行性和风险，解读当前截面。'],
  ['评价观点', '请评价我的市场观点：先列支持证据、反对证据、失效条件，再说明适合或不适合的期权表达。'],
  ['复核策略', '请复核当前策略预览，重点检查报价、价差、最大亏损、盈亏平衡、Greeks、期限和波动率风险。'],
  ['对比变化', '请对比两个截面，先确认时间与口径是否可比，再解释价格、IV、偏度、GEX、流动性和策略风险的变化。'],
]

function recordLabel(record) {
  const time = new Date(record.created_at).toLocaleString('zh-CN', { hour12: false })
  return `${record.symbol} · ${time} · ${record.kind}`
}

function MessageContent({ content, status }) {
  const text = content || status || ''
  return (
    <div className="assistant-markdown">
      <ReactMarkdown
        remarkPlugins={[remarkGfm]}
        components={{
          a: ({ children, ...props }) => (
            <a {...props} target="_blank" rel="noreferrer noopener">{children}</a>
          ),
        }}
      >
        {text}
      </ReactMarkdown>
    </div>
  )
}

export default function AssistantDock({
  currentSnapshotRef,
  currentSnapshotLabel,
  currentSnapshotReady,
  strategy,
  auditRecords,
  onAuditRefresh,
  onClose,
  onError,
}) {
  const [status, setStatus] = useState(null)
  const [sessions, setSessions] = useState([])
  const [session, setSession] = useState(null)
  const [messages, setMessages] = useState([])
  const [useCurrent, setUseCurrent] = useState(true)
  const [auditContextIds, setAuditContextIds] = useState([])
  const [auditDraft, setAuditDraft] = useState('')
  const [importDraft, setImportDraft] = useState('')
  const [prompt, setPrompt] = useState('')
  const [streaming, setStreaming] = useState(false)
  const [saved, setSaved] = useState(false)
  const [followOutput, setFollowOutput] = useState(true)
  const scrollRef = useRef(null)
  const followOutputRef = useRef(true)
  const pendingDeltaRef = useRef('')
  const flushTimerRef = useRef(null)
  const abortRef = useRef(null)
  const streamingRef = useRef(false)
  const waitTimersRef = useRef([])

  const snapshotRecords = useMemo(
    () => auditRecords.filter((record) => record.kind !== 'assistant_analysis'),
    [auditRecords],
  )
  const assistantRecords = useMemo(
    () => auditRecords.filter((record) => record.kind === 'assistant_analysis'),
    [auditRecords],
  )
  const contextRefs = useMemo(() => {
    const refs = []
    if (useCurrent && currentSnapshotReady && currentSnapshotRef) refs.push(currentSnapshotRef)
    for (const recordId of auditContextIds) refs.push({ kind: 'audit', record_id: recordId })
    return refs.slice(0, 2)
  }, [auditContextIds, currentSnapshotReady, currentSnapshotRef, useCurrent])

  const setFollowing = useCallback((next) => {
    followOutputRef.current = next
    setFollowOutput(next)
  }, [])

  const scrollToLatest = useCallback((behavior = 'auto') => {
    const element = scrollRef.current
    if (!element) return
    element.scrollTo({ top: element.scrollHeight, behavior })
  }, [])

  const resumeFollowing = useCallback(() => {
    setFollowing(true)
    requestAnimationFrame(() => scrollToLatest('smooth'))
  }, [scrollToLatest, setFollowing])

  const refreshSessions = useCallback(async () => {
    const data = await api('/api/assistant/sessions')
    setSessions(data)
    return data
  }, [])

  const openSession = useCallback(async (id) => {
    if (!id) return
    const data = await api(`/api/assistant/sessions/${id}`)
    setFollowing(true)
    setSession(data)
    setMessages(data.messages || [])
    setSaved(Boolean(data.imported_from))
    localStorage.setItem(ACTIVE_SESSION_KEY, id)
  }, [setFollowing])

  const createSession = useCallback(async () => {
    const data = await apiJson('/api/assistant/sessions', 'POST', { title: '新解盘' })
    setFollowing(true)
    setSession(data)
    setMessages([])
    setSaved(false)
    localStorage.setItem(ACTIVE_SESSION_KEY, data.id)
    await refreshSessions()
    return data
  }, [refreshSessions, setFollowing])

  useEffect(() => {
    let active = true
    Promise.all([api('/api/assistant/status'), refreshSessions()])
      .then(async ([nextStatus, nextSessions]) => {
        if (!active) return
        setStatus(nextStatus)
        const stored = localStorage.getItem(ACTIVE_SESSION_KEY)
        const candidate = stored
          ? nextSessions.find((item) => item.id === stored)
          : null
        if (candidate) {
          await openSession(candidate.id)
        } else {
          await createSession()
        }
      })
      .catch((reason) => active && onError(reason.message))
    return () => {
      active = false
      abortRef.current?.abort()
      if (flushTimerRef.current) window.clearTimeout(flushTimerRef.current)
      waitTimersRef.current.forEach((timer) => window.clearTimeout(timer))
    }
  }, [createSession, onError, openSession, refreshSessions])

  useEffect(() => {
    if (!followOutputRef.current) return undefined
    const frame = requestAnimationFrame(() => scrollToLatest())
    return () => cancelAnimationFrame(frame)
  }, [messages, scrollToLatest])

  const flushDelta = useCallback(() => {
    flushTimerRef.current = null
    const delta = pendingDeltaRef.current
    pendingDeltaRef.current = ''
    if (!delta) return
    setMessages((current) => {
      const next = [...current]
      const index = next.findLastIndex((item) => item.role === 'assistant' && item.streaming)
      if (index < 0) return next
      next[index] = {
        ...next[index],
        content: `${next[index].content}${delta}`,
        status: null,
      }
      return next
    })
  }, [])

  const sendMessage = useCallback(async (requestedPrompt) => {
    const content = (requestedPrompt ?? prompt).trim()
    if (!content || streamingRef.current) return
    if (!contextRefs.length) {
      onError('请先附加当前截面或一个收藏截面')
      return
    }
    setFollowing(true)
    streamingRef.current = true
    setStreaming(true)
    let localAssistantId = ''
    let timedOut = false
    try {
      let activeSession = session
      if (!activeSession) activeSession = await createSession()
      const now = Date.now()
      localAssistantId = `local-assistant-${now}`
      const localUser = {
        id: `local-user-${now}`,
        role: 'user',
        content,
        created_at: new Date().toISOString(),
      }
      const localAssistant = {
        id: localAssistantId,
        role: 'assistant',
        content: '',
        status: '正在冻结并读取截面…',
        created_at: new Date().toISOString(),
        streaming: true,
      }
      setMessages((current) => [...current, localUser, localAssistant])
      setPrompt('')
      setSaved(false)
      pendingDeltaRef.current = ''
      const updateStatus = (statusText) => {
        setMessages((current) => current.map((item) => (
          item.id === localAssistantId && item.streaming && !item.content
            ? { ...item, status: statusText }
            : item
        )))
      }
      waitTimersRef.current = [
        window.setTimeout(() => updateStatus('模型正在生成答案…'), 8_000),
        window.setTimeout(() => updateStatus('模型响应较慢，仍在等待…'), 30_000),
      ]
      const controller = new AbortController()
      abortRef.current = controller
      waitTimersRef.current.push(window.setTimeout(() => {
        timedOut = true
        controller.abort()
      }, 90_000))
      await apiEventStream(
        `/api/assistant/sessions/${activeSession.id}/messages`,
        {
          message: content,
          context_refs: contextRefs,
          strategy: strategy || null,
        },
        (event, payload) => {
          if (event === 'delta') {
            pendingDeltaRef.current += payload.content || ''
            if (!flushTimerRef.current) {
              flushTimerRef.current = window.setTimeout(flushDelta, 40)
            }
          }
          if (event === 'error') throw new Error(payload.detail || '助手返回错误')
        },
        { signal: controller.signal },
      )
      flushDelta()
      await openSession(activeSession.id)
      await refreshSessions()
    } catch (reason) {
      flushDelta()
      const detail = timedOut
        ? '请求超过 90 秒，已停止等待'
        : reason.name === 'AbortError'
          ? '请求已取消'
          : reason.message
      setMessages((current) => current.map((item) => (
        item.id === localAssistantId
          ? { ...item, role: 'error', content: `请求失败：${detail}`, status: null, streaming: false }
          : item
      )))
      if (reason.name !== 'AbortError' || timedOut) onError(detail)
    } finally {
      waitTimersRef.current.forEach((timer) => window.clearTimeout(timer))
      waitTimersRef.current = []
      abortRef.current = null
      streamingRef.current = false
      setStreaming(false)
    }
  }, [contextRefs, createSession, flushDelta, onError, openSession, prompt, refreshSessions, session, setFollowing, strategy])

  const addAuditContext = () => {
    if (!auditDraft || auditContextIds.includes(auditDraft)) return
    const limit = useCurrent && currentSnapshotReady ? 1 : 2
    setAuditContextIds((current) => [...current, auditDraft].slice(-limit))
    setAuditDraft('')
  }

  const deleteSession = async () => {
    if (!session || streaming) return
    await apiJson(`/api/assistant/sessions/${session.id}`, 'DELETE')
    localStorage.removeItem(ACTIVE_SESSION_KEY)
    setSession(null)
    setMessages([])
    await createSession()
  }

  const favorite = async () => {
    if (!session || !messages.length || streaming) return
    await apiJson(`/api/assistant/sessions/${session.id}/favorite`, 'POST')
    setSaved(true)
    await onAuditRefresh()
  }

  const importFavorite = async () => {
    if (!importDraft || streaming) return
    const imported = await apiJson('/api/assistant/sessions/import', 'POST', {
      audit_record_id: importDraft,
    })
    setFollowing(true)
    setSession(imported)
    setMessages(imported.messages || [])
    setSaved(true)
    localStorage.setItem(ACTIVE_SESSION_KEY, imported.id)
    setImportDraft('')
    await refreshSessions()
  }

  return (
    <aside className={`assistant-dock ${status?.enabled ? 'enabled' : 'disabled'}`} role="dialog" aria-label="截面解盘助手">
      <header className="assistant-header">
        <div><Bot size={17} /><span><strong>截面解盘助手</strong><small>{status?.enabled ? status.model : 'LLM 未配置'}</small></span></div>
        <div>
          <button title="新会话" onClick={createSession} disabled={streaming}><MessageSquarePlus size={15} /></button>
          <button title={saved ? '已收藏' : '收藏到审计账本'} onClick={favorite} disabled={!messages.length || streaming}>{saved ? <Check size={15} /> : <Bookmark size={15} />}</button>
          <button title="删除当前会话" onClick={deleteSession} disabled={streaming}><Trash2 size={15} /></button>
          <button title="隐藏助手" onClick={onClose}><X size={16} /></button>
        </div>
      </header>

      <div className="assistant-session-row">
        <History size={13} />
        <select value={session?.id || ''} onChange={(event) => openSession(event.target.value)}>
          <option value="">会话</option>
          {sessions.map((item) => <option key={item.id} value={item.id}>{item.title} · {item.message_count}</option>)}
        </select>
        <select value={importDraft} onChange={(event) => setImportDraft(event.target.value)}>
          <option value="">恢复收藏</option>
          {assistantRecords.map((record) => <option key={record.id} value={record.id}>{recordLabel(record)}</option>)}
        </select>
        <button title="恢复选中的收藏" onClick={importFavorite} disabled={!importDraft || streaming}><Plus size={14} /></button>
      </div>

      <section className="assistant-contexts" aria-label="附加截面">
        <div className="assistant-context-title"><span><Link2 size={12} />证据截面</span><b>{contextRefs.length}/2</b></div>
        <label className={currentSnapshotReady ? '' : 'disabled'}>
          <input
            type="checkbox"
            checked={useCurrent}
            disabled={!currentSnapshotReady}
            onChange={(event) => {
              setUseCurrent(event.target.checked)
              if (event.target.checked) setAuditContextIds((current) => current.slice(0, 1))
            }}
          />
          <span>{currentSnapshotReady ? currentSnapshotLabel : '当前截面尚未就绪'}</span>
        </label>
        {auditContextIds.map((recordId) => {
          const record = snapshotRecords.find((item) => item.id === recordId)
          return <div className="assistant-context-token" key={recordId}><span>{record ? recordLabel(record) : recordId}</span><button title="移除截面" onClick={() => setAuditContextIds((current) => current.filter((id) => id !== recordId))}><X size={12} /></button></div>
        })}
        <div className="assistant-context-add">
          <select value={auditDraft} onChange={(event) => setAuditDraft(event.target.value)}>
            <option value="">选择审计截面</option>
            {snapshotRecords.filter((record) => !auditContextIds.includes(record.id)).map((record) => <option key={record.id} value={record.id}>{recordLabel(record)}</option>)}
          </select>
          <button title="附加截面" onClick={addAuditContext} disabled={!auditDraft || contextRefs.length >= 2}><Plus size={14} /></button>
        </div>
      </section>

      {!status?.enabled && <div className="assistant-disabled">
        服务端未配置模型。设置 `OPTION_WORKSTATION_LLM_API_KEY` 与 `OPTION_WORKSTATION_LLM_MODEL` 后重启；测试可使用 mock 模式。
      </div>}

      <div className="assistant-quick">
        {QUICK_PROMPTS.map(([label, content]) => <button key={label} onClick={() => sendMessage(content)} disabled={!status?.enabled || streaming}>{label}</button>)}
      </div>

      <div className="assistant-message-stage">
        <div
          className="assistant-messages"
          ref={scrollRef}
          role="log"
          aria-live="polite"
          onScroll={(event) => {
            const element = event.currentTarget
            const atLatest = element.scrollHeight - element.scrollTop - element.clientHeight < 48
            if (atLatest !== followOutputRef.current) setFollowing(atLatest)
          }}
        >
          {!messages.length && <div className="assistant-empty"><Bot size={20} /><strong>附加截面后开始解盘</strong><span>助手会先检查数据质量，再解释定价、敞口、策略与失效条件。</span></div>}
          {messages.map((message) => <article key={message.id} className={`assistant-message ${message.role}${message.streaming ? ' streaming' : ''}`}>
            <span>{message.role === 'user' ? '你' : 'AI'}</span>
            <MessageContent content={message.content} status={message.status || (message.streaming ? '正在读取冻结截面…' : '')} />
          </article>)}
        </div>
        {!followOutput && (
          <button className="assistant-follow" type="button" title="回到最新回复" aria-label="回到最新回复" onClick={resumeFollowing}>
            <ArrowDown size={15} />
          </button>
        )}
      </div>

      <form className="assistant-composer" onSubmit={(event) => { event.preventDefault(); sendMessage() }}>
        <textarea
          value={prompt}
          maxLength={8000}
          placeholder="输入市场观点、策略假设或要复核的问题…"
          onChange={(event) => setPrompt(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === 'Enter' && !event.shiftKey) {
              event.preventDefault()
              sendMessage()
            }
          }}
        />
        <button type="submit" title="发送" disabled={!status?.enabled || !prompt.trim() || streaming}><Send size={16} /></button>
      </form>
      <footer>研究辅助 · 最多 2 个冻结截面 · 不调用交易接口</footer>
    </aside>
  )
}
