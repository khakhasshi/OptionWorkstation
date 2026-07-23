import { CheckCircle2, FileClock } from 'lucide-react'

export default function AuditPanel({ records, selectedId, onSelect, onCapture }) {
  return <div className="audit-panel-body">
    <div className="audit-toolbar"><span>追加式哈希账本</span><button onClick={onCapture}><FileClock size={13} />保存当前截面</button></div>
    <div className="audit-list">{records.length ? records.map((record) => <button className={selectedId === record.id ? 'active' : ''} key={record.id} onClick={() => onSelect(record)}>
      <CheckCircle2 size={13} /><span><b>{record.symbol} · {record.kind}</b><small>{new Date(record.created_at).toLocaleString('zh-CN', { hour12: false })} · {record.record_hash.slice(0, 10)}</small></span>
    </button>) : <div className="empty-state">尚无服务端审计记录</div>}</div>
  </div>
}
