import 'echarts-gl'
import Chart from './Chart'

export default function SurfaceChart(props) {
  return <Chart {...props} className={`surface-chart ${props.className || ''}`} incremental preserveView />
}
