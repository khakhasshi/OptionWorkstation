import { memo, useLayoutEffect, useRef } from 'react'
import { BarChart, CandlestickChart, HeatmapChart, LineChart, ScatterChart } from 'echarts/charts'
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
import { createChartEventBindings, createChartScheduler } from '../lib/chartScheduler'

echarts.use([
  BarChart,
  CandlestickChart,
  HeatmapChart,
  LineChart,
  ScatterChart,
  DataZoomComponent,
  GridComponent,
  LegendComponent,
  MarkLineComponent,
  TooltipComponent,
  VisualMapComponent,
  CanvasRenderer,
])

const NO_EVENTS = Object.freeze({})

function Chart({ option, className = '', onEvents = NO_EVENTS, incremental = false, preserveView = false, viewKey = '' }) {
  const ref = useRef(null)
  const scheduler = useRef(null)
  const bindings = useRef(null)
  const handlers = useRef(onEvents)
  const settings = useRef({ preserveView })
  const camera = useRef(null)
  const zoom = useRef(null)
  const optionVersion = useRef(0)

  useLayoutEffect(() => {
    const node = ref.current
    if (!node) return undefined
    let renderMs = 0
    let resizeCount = 0
    const controller = createChartScheduler({
      requestFrame: (callback) => requestAnimationFrame(callback),
      cancelFrame: (frame) => cancelAnimationFrame(frame),
      create: (width, height) => {
        const instance = echarts.init(node, null, { renderer: 'canvas', width, height })
        instance.on('grid3dcamerachanged', (event) => {
          if (!settings.current.preserveView) return
          camera.current = { alpha: event.alpha, beta: event.beta, distance: event.distance, center: event.center }
          node.dataset.cameraAlpha = String(event.alpha)
          node.dataset.cameraBeta = String(event.beta)
          node.dataset.cameraDistance = String(event.distance)
        })
        instance.on('datazoom', () => {
          zoom.current = instance.getOption().dataZoom?.map(({ id, start, end }) => ({ id, start, end })) || null
        })
        bindings.current = createChartEventBindings(instance)
        bindings.current.update(handlers.current)
        return instance
      },
      render: (instance, input, reset) => {
        const started = performance.now()
        if (reset) {
          camera.current = null
          zoom.current = null
          instance.clear()
          delete node.dataset.cameraAlpha
          delete node.dataset.cameraBeta
          delete node.dataset.cameraDistance
        }
        instance.dispatchAction({ type: 'hideTip' })
        if (!input.option) {
          instance.clear()
        } else {
          let nextOption = input.option
          if (input.preserveView && camera.current && nextOption.grid3D) {
            nextOption = {
              ...nextOption,
              grid3D: {
                ...nextOption.grid3D,
                viewControl: { ...nextOption.grid3D.viewControl, ...camera.current },
              },
            }
          }
          if (zoom.current && nextOption.dataZoom) {
            const dataZoom = Array.isArray(nextOption.dataZoom) ? nextOption.dataZoom : [nextOption.dataZoom]
            nextOption = {
              ...nextOption,
              dataZoom: dataZoom.map((item, index) => {
                const saved = item.id == null ? zoom.current[index] : zoom.current.find((value) => value.id === item.id)
                return saved ? { ...item, start: saved.start, end: saved.end } : item
              }),
            }
          }
          instance.setOption(nextOption, { notMerge: !input.incremental, lazyUpdate: false, silent: input.incremental })
          optionVersion.current += 1
          node.dataset.optionVersion = String(optionVersion.current)
        }
        const elapsed = performance.now() - started
        renderMs += elapsed
        node.dataset.renderMs = renderMs.toFixed(3)
        node.dataset.lastRenderMs = elapsed.toFixed(3)
      },
      resize: (instance, width, height) => {
        instance.resize({ width, height })
        resizeCount += 1
        node.dataset.resizeCount = String(resizeCount)
      },
      dispose: (instance) => {
        bindings.current?.dispose()
        bindings.current = null
        instance.dispose()
      },
    })
    scheduler.current = controller
    controller.setSize(node.clientWidth, node.clientHeight)
    const resizeObserver = new ResizeObserver(() => controller.setSize(node.clientWidth, node.clientHeight))
    resizeObserver.observe(node)
    const intersectionObserver = typeof IntersectionObserver === 'function'
      ? new IntersectionObserver((entries) => {
          const entry = entries.at(-1)
          if (entry) controller.setVisible(entry.isIntersecting)
        })
      : null
    if (intersectionObserver) intersectionObserver.observe(node)
    else controller.setVisible(true)
    return () => {
      intersectionObserver?.disconnect()
      resizeObserver.disconnect()
      controller.dispose()
      scheduler.current = null
    }
  }, [])

  useLayoutEffect(() => {
    handlers.current = onEvents
    bindings.current?.update(onEvents)
  }, [onEvents])

  useLayoutEffect(() => {
    settings.current = { preserveView }
    if (!preserveView) camera.current = null
    scheduler.current?.update({ option, incremental, preserveView, viewKey })
  }, [option, incremental, preserveView, viewKey])

  return <div ref={ref} className={`chart ${className}`} />
}

export default memo(Chart)
