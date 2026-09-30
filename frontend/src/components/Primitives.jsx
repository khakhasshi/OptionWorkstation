import { ChevronDown, ChevronUp } from 'lucide-react'
import { useId, useState } from 'react'

export function Metric({ label, value, tone = '', detail = '' }) {
  return <div className="metric" title={detail}><span>{label}</span><strong className={tone}>{value ?? '--'}</strong>{detail && <small>{detail}</small>}</div>
}

export function LiveReadout({ label, value, tone = '' }) {
  return <div className={`live-readout ${tone}`}><span>{label}</span><strong>{value}</strong></div>
}

export function Panel({ id, title, icon, tools, className = '', children, collapsible = true }) {
  const generatedId = useId()
  const panelId = id ? `panel-${id}` : generatedId
  const headingId = `${panelId}-heading`
  const bodyId = `${panelId}-body`
  const storageKey = `option-workstation-panel-${id || className}`
  const [collapsed, setCollapsed] = useState(() => collapsible && localStorage.getItem(storageKey) === 'collapsed')
  const toggle = () => {
    const next = !collapsed
    setCollapsed(next)
    localStorage.setItem(storageKey, next ? 'collapsed' : 'open')
  }
  return <section className={`workspace-panel analytic-panel ${className} ${collapsed ? 'collapsed' : ''}`} aria-labelledby={headingId}>
    <div className="panel-header">
      <div>{icon}<h2 id={headingId} className="panel-heading">{title}</h2></div>
      <div className="panel-actions">{tools && <div className="panel-tools">{tools}</div>}{collapsible && <button className="panel-collapse" onClick={toggle} title={collapsed ? `展开${title}` : `折叠${title}`} aria-label={collapsed ? `展开${title}` : `折叠${title}`} aria-expanded={!collapsed} aria-controls={bodyId}>{collapsed ? <ChevronDown size={16} aria-hidden="true" /> : <ChevronUp size={16} aria-hidden="true" />}</button>}</div>
    </div>
    <div className="panel-body" id={bodyId} hidden={collapsed}>{!collapsed && children}</div>
  </section>
}
