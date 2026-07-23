import fs from 'node:fs/promises'
import path from 'node:path'
import puppeteer from '../frontend/node_modules/puppeteer-core/lib/puppeteer/puppeteer-core.js'

const baseUrl = (process.argv[2] || 'http://127.0.0.1:7311').replace(/\/$/, '')
const artifactDir = path.resolve('artifacts/browser-smoke')
await fs.mkdir(artifactDir, { recursive: true })

const browser = await puppeteer.launch({
  executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  headless: true,
  args: [
    '--enable-webgl',
    '--enable-unsafe-swiftshader',
    '--ignore-gpu-blocklist',
    '--no-sandbox',
  ],
})

const errors = []
const page = await browser.newPage()
page.on('console', (message) => {
  if (message.type() === 'error') errors.push(`console: ${message.text()}`)
})
page.on('pageerror', (error) => errors.push(`page: ${error.message}`))

try {
  await page.setViewport({ width: 1600, height: 1000, deviceScaleFactor: 1 })
  await page.goto(`${baseUrl}/?mode=replay`, { waitUntil: 'networkidle2' })
  await page.waitForSelector('.research-workspace', { timeout: 20_000 })
  const desktopOverflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)
  if (desktopOverflow > 1) throw new Error(`desktop horizontal overflow: ${desktopOverflow}px`)
  await page.screenshot({ path: path.join(artifactDir, 'desktop-replay.png') })
  await page.waitForSelector('.chain-table .add-leg', { timeout: 20_000 })
  await page.$eval('.chain-table .add-leg', (button) => button.click())
  await page.waitForSelector('.strategy-leg', { timeout: 10_000 })
  await page.$eval('.strategy-toolbar button', (button) => button.click())
  await page.waitForSelector('.risk-metrics', { timeout: 20_000 })
  const scenarioCells = await page.$$eval('.scenario-matrix td', (cells) => cells.length)
  if (scenarioCells !== 15) throw new Error(`strategy scenario matrix has ${scenarioCells} cells`)

  await page.waitForSelector('.surface-chart canvas', { timeout: 30_000 })
  const surface = await page.$('.surface-chart')
  await surface.evaluate((element) => element.scrollIntoView({ block: 'center' }))

  const canvas = await page.$('.surface-chart canvas')
  const canvasPng = await canvas.screenshot({ encoding: 'binary' })
  if (canvasPng.length < 5_000) throw new Error('3D surface canvas appears blank')
  await page.screenshot({ path: path.join(artifactDir, 'replay-surface-before-drag.png') })
  const box = await surface.boundingBox()
  if (!box || box.width < 100 || box.height < 100) throw new Error('3D surface has no usable bounds')

  const initialCamera = await surface.evaluate((element) => ({
    alpha: Number(element.dataset.cameraAlpha),
    beta: Number(element.dataset.cameraBeta),
    distance: Number(element.dataset.cameraDistance),
  }))
  await page.mouse.move(box.x + box.width * 0.52, box.y + box.height * 0.48)
  await page.mouse.down({ button: 'left' })
  await page.mouse.move(box.x + box.width * 0.72, box.y + box.height * 0.67, { steps: 14 })
  await page.mouse.up({ button: 'left' })
  await new Promise((resolve) => setTimeout(resolve, 500))

  const afterDrag = await surface.evaluate((element) => ({
    alpha: Number(element.dataset.cameraAlpha),
    beta: Number(element.dataset.cameraBeta),
    distance: Number(element.dataset.cameraDistance),
    version: Number(element.dataset.optionVersion),
  }))
  if (!Number.isFinite(afterDrag.alpha) || !Number.isFinite(afterDrag.beta)) {
    throw new Error('3D camera change event was not observed')
  }
  if (Math.abs(afterDrag.alpha - initialCamera.alpha) < 0.1 && Math.abs(afterDrag.beta - initialCamera.beta) < 0.1) {
    throw new Error(`3D drag did not move the camera: ${JSON.stringify({ initialCamera, afterDrag, box })}`)
  }

  for (let step = 0; step < 5; step += 1) {
    await page.$eval('button[title="下一帧"]', (button) => button.click())
    await new Promise((resolve) => setTimeout(resolve, 120))
  }
  await page.waitForFunction(
    (version) => Number(document.querySelector('.surface-chart')?.dataset.optionVersion) > version,
    { timeout: 20_000 },
    afterDrag.version,
  )
  const afterRefresh = await surface.evaluate((element) => ({
    alpha: Number(element.dataset.cameraAlpha),
    beta: Number(element.dataset.cameraBeta),
    distance: Number(element.dataset.cameraDistance),
    version: Number(element.dataset.optionVersion),
  }))
  const cameraDrift = Math.max(
    Math.abs(afterRefresh.alpha - afterDrag.alpha),
    Math.abs(afterRefresh.beta - afterDrag.beta),
    Math.abs(afterRefresh.distance - afterDrag.distance),
  )
  if (cameraDrift > 0.05) throw new Error(`3D camera reset after refresh; drift=${cameraDrift}`)
  await page.screenshot({ path: path.join(artifactDir, 'replay-surface-rotated.png') })

  await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 })
  await page.goto(`${baseUrl}/?mode=replay`, { waitUntil: 'networkidle2' })
  await page.waitForSelector('.research-workspace', { timeout: 20_000 })
  const mobileOverflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)
  if (mobileOverflow > 1) throw new Error(`mobile horizontal overflow: ${mobileOverflow}px`)
  await page.screenshot({ path: path.join(artifactDir, 'mobile-replay.png') })

  if (errors.length) throw new Error(errors.join('\n'))
  console.log(JSON.stringify({
    ok: true,
    desktopOverflow,
    mobileOverflow,
    initialCamera,
    afterDrag,
    afterRefresh,
    cameraDrift,
    artifacts: artifactDir,
  }, null, 2))
} finally {
  await browser.close()
}
