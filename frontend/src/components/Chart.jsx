import { useEffect, useLayoutEffect, useRef } from 'react'
import { BarChart, CandlestickChart, HeatmapChart, LineChart } from 'echarts/charts'
import {
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkLineComponent,
  TooltipComponent,
  VisualMapComponent,
} from 'echarts/components'
import * as echarts from 'echarts/core'
import { CanvasRenderer } from 'echarts/renderers'

echarts.use([
  BarChart,
  CandlestickChart,
  HeatmapChart,
  LineChart,
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkLineComponent,
  TooltipComponent,
  VisualMapComponent,
  CanvasRenderer,
])

export default function Chart({ option, className = '', onEvents = {}, incremental = false, preserveView = false, viewKey = '' }) {
  const ref = useRef(null)
  const chart = useRef(null)
  const camera = useRef(null)
  const previousViewKey = useRef(viewKey)
  const optionVersion = useRef(0)

  useLayoutEffect(() => {
    if (!ref.current) return undefined
    chart.current = echarts.init(ref.current, null, { renderer: 'canvas' })
    const captureCamera = (event) => {
      camera.current = {
        alpha: event.alpha,
        beta: event.beta,
        distance: event.distance,
        center: event.center,
      }
      if (ref.current) {
        ref.current.dataset.cameraAlpha = String(event.alpha)
        ref.current.dataset.cameraBeta = String(event.beta)
        ref.current.dataset.cameraDistance = String(event.distance)
      }
    }
    if (preserveView) chart.current.on('grid3dcamerachanged', captureCamera)
    const resize = new ResizeObserver(() => chart.current?.resize())
    resize.observe(ref.current)
    return () => {
      resize.disconnect()
      if (preserveView) chart.current?.off('grid3dcamerachanged', captureCamera)
      chart.current?.dispose()
    }
  }, [preserveView])

  useLayoutEffect(() => {
    if (previousViewKey.current !== viewKey) {
      previousViewKey.current = viewKey
      camera.current = null
      chart.current?.dispatchAction({ type: 'hideTip' })
      chart.current?.clear()
      if (ref.current) {
        delete ref.current.dataset.cameraAlpha
        delete ref.current.dataset.cameraBeta
        delete ref.current.dataset.cameraDistance
      }
    }
  }, [viewKey])

  useLayoutEffect(() => {
    const instance = chart.current
    if (!instance) return
    instance.dispatchAction({ type: 'hideTip' })
    if (!option) {
      instance.clear()
      return
    }
    const nextOption = preserveView && camera.current && option.grid3D
      ? {
          ...option,
          grid3D: {
            ...option.grid3D,
            viewControl: { ...option.grid3D.viewControl, ...camera.current },
          },
        }
      : option
    instance.setOption(nextOption, {
      notMerge: !incremental,
      lazyUpdate: false,
      silent: incremental,
    })
    optionVersion.current += 1
    if (ref.current) ref.current.dataset.optionVersion = String(optionVersion.current)
  }, [option, incremental, preserveView, viewKey])

  useEffect(() => {
    const instance = chart.current
    if (!instance) return undefined
    Object.entries(onEvents).forEach(([name, handler]) => instance.on(name, handler))
    return () => Object.entries(onEvents).forEach(([name, handler]) => instance.off(name, handler))
  }, [onEvents])

  return <div ref={ref} className={`chart ${className}`} />
}
