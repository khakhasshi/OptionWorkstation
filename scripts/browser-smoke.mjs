import fs from 'node:fs/promises'
import path from 'node:path'
import puppeteer from '../frontend/node_modules/puppeteer-core/lib/puppeteer/puppeteer-core.js'

const baseUrl = (process.argv[2] || 'http://127.0.0.1:7311').replace(/\/$/, '')
const artifactDir = path.resolve('artifacts/browser-smoke')
await fs.mkdir(artifactDir, { recursive: true })

const browser = await puppeteer.launch({
  executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  headless: true,
  args: ['--enable-webgl', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--no-sandbox'],
})

const errors = []
const page = await browser.newPage()
page.on('console', (message) => {
  if (message.type() === 'error') errors.push(`console: ${message.text()}`)
})
page.on('pageerror', (error) => errors.push(`page: ${error.message}`))

async function clickText(selector, label) {
  await page.$$eval(selector, (buttons, text) => {
    const button = buttons.find((element) => element.textContent.trim() === text)
    if (!button || button.disabled) throw new Error(`Unavailable button: ${text}`)
    button.click()
  }, label)
}

async function workspace(label) {
  await clickText('.layout-switch button', label)
  await page.waitForFunction((text) => document.querySelector('.layout-switch button[aria-pressed="true"]')?.textContent.trim() === text, {}, label)
}

async function assertOverview() {
  await workspace('总览')
  const hiddenPanels = await page.$$eval('.surface-panel, .chain-panel, .surface-chart, .chain-table', (nodes) => nodes.length)
  if (hiddenPanels) throw new Error(`Overview mounted ${hiddenPanels} inactive surface/chain nodes`)
}

async function scrollToTop() {
  await page.evaluate(() => {
    document.querySelector('.research-workspace').scrollTop = 0
    window.scrollTo(0, 0)
  })
}

async function layoutMetrics(name) {
  await page.$$eval('.research-workspace .panel-collapse[aria-expanded="false"]', (buttons) => buttons.forEach((button) => button.click()))
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(() => requestAnimationFrame(resolve))))
  const result = await page.evaluate(() => {
    const bounds = (selector) => {
      const node = document.querySelector(selector)
      if (!node) return null
      const { left, top, right, bottom, width, height } = node.getBoundingClientRect()
      return { left, top, right, bottom, width, height }
    }
    const main = document.querySelector('.research-workspace')
    const mainRect = main.getBoundingClientRect()
    const expandedPanels = [...main.querySelectorAll('.workspace-panel:not(.collapsed)')]
    const panelBottom = Math.max(0, ...expandedPanels.map((panel) => panel.getBoundingClientRect().bottom - mainRect.top - main.clientTop + main.scrollTop))
    const visible = (node) => {
      let { left, right, top, bottom } = node.getBoundingClientRect()
      left = Math.max(left, 0); right = Math.min(right, innerWidth)
      top = Math.max(top, 0); bottom = Math.min(bottom, innerHeight)
      for (let parent = node.parentElement; parent; parent = parent.parentElement) {
        const style = getComputedStyle(parent)
        const rect = parent.getBoundingClientRect()
        if (/(auto|scroll|hidden|clip)/.test(style.overflowX)) {
          left = Math.max(left, rect.left + parent.clientLeft)
          right = Math.min(right, rect.left + parent.clientLeft + parent.clientWidth)
        }
        if (/(auto|scroll|hidden|clip)/.test(style.overflowY)) {
          top = Math.max(top, rect.top + parent.clientTop)
          bottom = Math.min(bottom, rect.top + parent.clientTop + parent.clientHeight)
        }
      }
      return right > left && bottom > top
    }
    const clippedChildren = []
    for (const panel of expandedPanels) {
      const rect = panel.getBoundingClientRect()
      const style = getComputedStyle(panel)
      const left = rect.left + panel.clientLeft + Number.parseFloat(style.paddingLeft)
      const right = rect.left + panel.clientLeft + panel.clientWidth - Number.parseFloat(style.paddingRight)
      for (const child of panel.querySelectorAll('.chart, .chart canvas, .market-heading, .panel-header, .panel-body, .compact-metrics, .oi-balance, .quality-banner, .snapshot-clock, .vol-provenance')) {
        const childRect = child.getBoundingClientRect()
        if (childRect.width <= 0 || childRect.height <= 0) continue
        // Offscreen canvases intentionally defer resize until re-entry. Their
        // responsive chart containers must still fit, even while offscreen.
        if (child.tagName === 'CANVAS' && !visible(child)) continue
        if (childRect.left < left - 1 || childRect.right > right + 1) {
          clippedChildren.push({ panel: panel.className, child: child.className || child.tagName, width: childRect.width, panelInnerWidth: right - left, left: childRect.left - left, right: childRect.right - right })
        }
      }
    }
    return {
      overflow: document.documentElement.scrollWidth - innerWidth,
      width: innerWidth, height: innerHeight,
      nav: bounds('.layout-nav'), dock: bounds('.playback-dock'),
      workspaceButtons: [...document.querySelectorAll('.layout-switch button')].map((button) => {
        const { left, right, width } = button.getBoundingClientRect()
        return { left, right, width }
      }),
      play: bounds('.playback-dock .play'), timeline: bounds('input[aria-label="回放进度"]'),
      clippedChildren,
      main: { scrollHeight: main.scrollHeight, clientHeight: main.clientHeight, expandedPanels: expandedPanels.length, requiredHeight: panelBottom + Number.parseFloat(getComputedStyle(main).paddingBottom) },
      smileChart: bounds('.smile-panel:not(.collapsed) .chart'),
    }
  })
  if (result.overflow > 1) throw new Error(`${name} horizontal overflow: ${result.overflow}px`)
  for (const key of ['nav', 'dock', 'play', 'timeline']) {
    const rect = result[key]
    if (!rect || rect.width <= 0 || rect.height <= 0 || rect.left < -1 || rect.right > result.width + 1 || rect.top < -1 || rect.bottom > result.height + 1) {
      throw new Error(`${name} ${key} is not fully visible: ${JSON.stringify(rect)}`)
    }
  }
  if (result.workspaceButtons.length !== 4 || result.workspaceButtons.some((rect) => rect.width <= 0 || rect.left < -1 || rect.right > result.width + 1)) {
    throw new Error(`${name} workspace navigation overflows`)
  }
  if (result.clippedChildren.length) throw new Error(`${name} panel content is clipped horizontally: ${JSON.stringify(result.clippedChildren)}`)
  if (result.width > 900 && result.main.scrollHeight + 2 < result.main.requiredHeight) {
    throw new Error(`${name} workspace scroll area omits expanded panels: ${JSON.stringify(result.main)}`)
  }
  if (result.smileChart && result.smileChart.height < 280) throw new Error(`${name} smile plot body is shorter than 280px: ${result.smileChart.height}`)
  return result
}

try {
  await page.setViewport({ width: 1600, height: 1000, deviceScaleFactor: 1 })
  await page.goto(`${baseUrl}/?mode=replay`, { waitUntil: 'networkidle2' })
  await page.waitForSelector('.research-workspace', { timeout: 20_000 })
  await assertOverview()
  await scrollToTop()
  const desktop = await layoutMetrics('desktop overview')
  await page.screenshot({ path: path.join(artifactDir, 'desktop-replay.png') })

  await workspace('交易')
  await page.waitForSelector('.chain-table .add-leg', { timeout: 20_000 })
  await page.$eval('.chain-panel', (element) => element.scrollIntoView({ block: 'center' }))
  const virtualChain = await page.evaluate(() => {
    const scroller = document.querySelector('.virtual-chain')
    return {
      total: Number.parseInt(document.querySelector('.chain-filters b').textContent, 10),
      mounted: document.querySelectorAll('.chain-data-row').length,
      height: scroller.clientHeight,
    }
  })
  const rowBudget = Math.ceil(Math.max(32, virtualChain.height - 64) / 32) + 13
  if (virtualChain.mounted > rowBudget) throw new Error(`Chain exceeds visible-row budget: ${JSON.stringify(virtualChain)}`)
  if (virtualChain.total <= virtualChain.mounted) {
    throw new Error(`Virtualization smoke requires a chain larger than one window: ${JSON.stringify(virtualChain)}`)
  }
  await page.focus('.chain-data-row[tabindex="0"]')
  await page.keyboard.press('End')
  await page.waitForFunction(() => {
    const total = Number.parseInt(document.querySelector('.chain-filters b').textContent, 10)
    return document.activeElement?.matches('.chain-data-row') && Number(document.activeElement.dataset.rowIndex) === total - 1
  })
  const atEnd = await page.$eval('.virtual-chain', (node) => ({ top: node.scrollTop, max: node.scrollHeight - node.clientHeight }))
  if (Math.abs(atEnd.top - atEnd.max) > 2) throw new Error(`End did not reach last strike: ${JSON.stringify(atEnd)}`)
  // Repeating a boundary key must not leave a deferred focus request that steals a filter's focus.
  await page.keyboard.press('End')
  await page.focus('.chain-filters input')
  await page.$eval('.chain-filters input', (input) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, '1')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await page.waitForFunction(() => document.querySelector('.chain-filters input')?.value === '1')
  await page.evaluate(() => new Promise((resolve) => requestAnimationFrame(resolve)))
  if (!await page.$eval('.chain-filters input', (input) => document.activeElement === input)) throw new Error('Chain filter lost keyboard focus after shrinking rows')
  await page.$eval('.chain-filters input', (input) => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set.call(input, '35')
    input.dispatchEvent(new Event('input', { bubbles: true }))
  })
  await page.waitForSelector('.chain-data-row[tabindex="0"]')
  await page.focus('.chain-data-row[tabindex="0"]')
  await page.keyboard.press('Home')
  await page.waitForFunction(() => document.activeElement?.matches('.chain-data-row') && document.activeElement.dataset.rowIndex === '0' && document.querySelector('.virtual-chain').scrollTop === 0)
  await page.keyboard.press('PageDown')
  await page.waitForFunction(() => document.activeElement?.matches('.chain-data-row') && Number(document.activeElement.dataset.rowIndex) > 0)
  await page.keyboard.press('Home')
  await page.waitForFunction(() => document.activeElement?.dataset.rowIndex === '0')

  await page.$eval('.chain-table .add-leg', (button) => button.click())
  await page.waitForSelector('.strategy-leg', { timeout: 10_000 })
  await page.$eval('.strategy-toolbar button', (button) => button.click())
  await page.waitForSelector('.risk-metrics', { timeout: 20_000 })
  const scenarioCells = await page.$$eval('.scenario-matrix td', (cells) => cells.length)
  if (scenarioCells !== 15) throw new Error(`strategy scenario matrix has ${scenarioCells} cells`)

  const dateSelector = 'select[aria-label="回放日期"]'
  const selectedDate = await page.$eval(dateSelector, (select) => select.value)
  const availableDates = await page.$$eval(`${dateSelector} option`, (options) => options.map((option) => option.value))
  const alternateDate = availableDates.find((value) => value !== selectedDate)
  if (alternateDate) {
    await page.select(dateSelector, alternateDate)
    await page.waitForFunction((date) => document.querySelector('.market-heading .eyebrow')?.textContent?.includes(date), { timeout: 20_000 }, alternateDate)
    await page.waitForSelector('.chain-table .add-leg', { timeout: 20_000 })
    if (errors.length) throw new Error(`date switch regression: ${errors.join('\n')}`)
  }

  await workspace('波动率')
  const default2d = await page.$$eval('[aria-label="曲面视图"] button', (buttons) => buttons.find((button) => button.textContent.trim() === '2D')?.getAttribute('aria-pressed') === 'true')
  if (!default2d || await page.$('.surface-chart')) throw new Error('Volatility workspace must initially use the 2D surface')
  // IntersectionObserver initializes charts only after their panel is visible.
  await page.$eval('.surface-panel', (element) => element.scrollIntoView({ block: 'center' }))
  await page.waitForSelector('.surface-panel .chart canvas', { timeout: 30_000 })
  await clickText('[aria-label="曲面视图"] button', '3D')
  await page.waitForSelector('.surface-chart', { timeout: 30_000 })
  const surface = await page.$('.surface-chart')
  await surface.evaluate((element) => element.scrollIntoView({ block: 'center' }))
  await page.waitForSelector('.surface-chart canvas', { timeout: 30_000 })
  await page.waitForFunction(() => Number(document.querySelector('.surface-chart')?.dataset.optionVersion) > 0)
  const canvas = await page.$('.surface-chart canvas')
  const canvasPng = await canvas.screenshot({ encoding: 'binary' })
  if (canvasPng.length < 5_000) throw new Error('3D surface canvas appears blank')
  await page.screenshot({ path: path.join(artifactDir, 'replay-surface-before-drag.png') })
  const box = await surface.boundingBox()
  if (!box || box.width < 100 || box.height < 100) throw new Error('3D surface has no usable bounds')

  const readCamera = (element) => ({
    alpha: Number(element.dataset.cameraAlpha), beta: Number(element.dataset.cameraBeta),
    distance: Number(element.dataset.cameraDistance), version: Number(element.dataset.optionVersion),
  })
  const initialCamera = await surface.evaluate(readCamera)
  await page.mouse.move(box.x + box.width * 0.52, box.y + box.height * 0.48)
  await page.mouse.down({ button: 'left' })
  await page.mouse.move(box.x + box.width * 0.72, box.y + box.height * 0.67, { steps: 14 })
  await page.mouse.up({ button: 'left' })
  await new Promise((resolve) => setTimeout(resolve, 500))
  const afterDrag = await surface.evaluate(readCamera)
  if (![afterDrag.alpha, afterDrag.beta, afterDrag.distance].every(Number.isFinite)) throw new Error('3D camera change event was not observed')
  if (Number.isFinite(initialCamera.alpha) && Number.isFinite(initialCamera.beta)
    && Math.abs(afterDrag.alpha - initialCamera.alpha) < 0.1 && Math.abs(afterDrag.beta - initialCamera.beta) < 0.1) {
    throw new Error(`3D drag did not move the camera: ${JSON.stringify({ initialCamera, afterDrag, box })}`)
  }
  const afterDragPng = await canvas.screenshot({ encoding: 'binary' })
  if (Buffer.from(canvasPng).equals(Buffer.from(afterDragPng))) throw new Error('3D drag did not change the rendered surface')

  for (let step = 0; step < 5; step += 1) {
    await page.$eval('button[title="下一帧"]', (button) => button.click())
    await new Promise((resolve) => setTimeout(resolve, 120))
  }
  await page.waitForFunction((version) => Number(document.querySelector('.surface-chart')?.dataset.optionVersion) > version, { timeout: 20_000 }, afterDrag.version)
  const afterRefresh = await surface.evaluate(readCamera)
  if (![afterRefresh.alpha, afterRefresh.beta, afterRefresh.distance].every(Number.isFinite)) throw new Error('Camera state missing after a replay update')
  const cameraDrift = Math.max(Math.abs(afterRefresh.alpha - afterDrag.alpha), Math.abs(afterRefresh.beta - afterDrag.beta), Math.abs(afterRefresh.distance - afterDrag.distance))
  if (cameraDrift > 0.05) throw new Error(`3D camera reset after refresh; drift=${cameraDrift}`)
  await page.screenshot({ path: path.join(artifactDir, 'replay-surface-rotated.png') })

  const responsive = {}
  await assertOverview()
  // Initialize each overview chart before shrinking this same page, so an old
  // canvas's intrinsic size cannot be hidden by rebuilding it after a reload.
  for (const chart of await page.$$('.research-workspace .chart')) {
    await chart.evaluate((element) => element.scrollIntoView({ block: 'center' }))
    await chart.waitForSelector('canvas', { timeout: 20_000 })
  }
  await scrollToTop()
  responsive.desktopResizeBase = await layoutMetrics('desktop resize baseline')
  for (const [name, width, height] of [['medium', 1024, 900], ['mobile', 390, 844]]) {
    await page.setViewport({ width, height, deviceScaleFactor: 1 })
    await assertOverview()
    await scrollToTop()
    const overview = await layoutMetrics(`${name} overview`)
    responsive[name] = { overview }
    await page.screenshot({ path: path.join(artifactDir, `${name}-replay.png`) })
  }
  for (const [name, width, height] of [['medium', 1024, 900], ['mobile', 390, 844]]) {
    await page.setViewport({ width, height, deviceScaleFactor: 1 })
    await workspace('交易')
    await page.waitForSelector('.chain-table .add-leg', { timeout: 20_000 })
    await scrollToTop()
    const trade = await layoutMetrics(`${name} trade`)
    responsive[name].trade = trade
    await page.screenshot({ path: path.join(artifactDir, `${name}-trade.png`) })
  }

  if (errors.length) throw new Error(errors.join('\n'))
  console.log(JSON.stringify({ ok: true, desktop, virtualChain, scenarioCells, responsive, initialCamera, afterDrag, afterRefresh, cameraDrift, artifacts: artifactDir }, null, 2))
} finally {
  await browser.close()
}
