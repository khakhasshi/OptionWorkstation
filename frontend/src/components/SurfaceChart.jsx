import { memo } from 'react'
import 'echarts-gl'
import Chart from './Chart'

function SurfaceChart(props) {
  return <Chart {...props} className={`surface-chart ${props.className || ''}`} incremental preserveView />
}

export default memo(SurfaceChart)
