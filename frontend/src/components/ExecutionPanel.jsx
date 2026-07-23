import { RefreshCw, ShieldAlert, Trash2 } from 'lucide-react'

export default function ExecutionPanel({ account, orders, onRefresh, onCancel }) {
  return <div className="execution-body">
    <div className="account-strip">
      <div><span>账户</span><strong>{account?.account_type || '未读取'}</strong></div>
      <div><span>Buying Power</span><strong>{account?.buy_power || '--'} {account?.currency || ''}</strong></div>
      <div><span>执行</span><strong className={account?.execution_enabled ? 'ok-text' : 'warning-text'}>{account?.execution_enabled ? 'Paper Enabled' : 'Locked'}</strong></div>
      <button onClick={onRefresh} title="刷新账户与订单"><RefreshCw size={14} /></button>
    </div>
    {!account?.paper_account && <div className="execution-warning"><ShieldAlert size={14} />仅识别为 paper account 后才开放订单执行</div>}
    <div className="order-list">{orders?.length ? orders.map((order) => <div className="order-row" key={order.order_id}>
      <span className={String(order.side).toLowerCase().includes('buy') ? 'call' : 'put'}>{String(order.side)}</span>
      <b>{order.symbol}</b><span>{String(order.status)}</span><span>{String(order.quantity)} @ {order.price ?? 'MKT'}</span>
      <button title="撤销订单" disabled={!account?.execution_enabled} onClick={() => onCancel(order.order_id)}><Trash2 size={13} /></button>
    </div>) : <div className="empty-state">今日暂无订单</div>}</div>
  </div>
}
