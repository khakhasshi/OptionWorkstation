import { ChevronDown, ChevronUp } from 'lucide-react'
import { useState } from 'react'

export function Metric({ label, value, tone = '', detail = '' }) {
  return <div className="metric" title={detail}><span>{label}</span><strong className={tone}>{value ?? '--'}</strong>{detail && <small>{detail}</small>}</div>
}

export function LiveReadout({ label, value, tone = '' }) {
  return <div className={`live-readout ${tone}`}><span>{label}</span><strong>{value}</strong></div>
}

export function Panel({ id, title, icon, tools, className = '', children, collapsible = true }) {
  const storageKey = `option-workstation-panel-${id || className}`
  const [collapsed, setCollapsed] = useState(() => collapsible && localStorage.getItem(storageKey) === 'collapsed')
  const toggle = () => {
    const next = !collapsed
    setCollapsed(next)
    localStorage.setItem(storageKey, next ? 'collapsed' : 'open')
  }
  return <section className={`workspace-panel analytic-panel ${className} ${collapsed ? 'collapsed' : ''}`}>
    <div className="panel-header">
      <div>{icon}<strong>{title}</strong></div>
      <div className="panel-actions">{tools && <div className="panel-tools">{tools}</div>}{collapsible && <button className="panel-collapse" onClick={toggle} title={collapsed ? '展开' : '折叠'}>{collapsed ? <ChevronDown size={14} /> : <ChevronUp size={14} />}</button>}</div>
    </div>
    {!collapsed && <div className="panel-body">{children}</div>}
  </section>
}
