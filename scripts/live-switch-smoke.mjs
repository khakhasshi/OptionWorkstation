import fs from 'node:fs/promises'
import path from 'node:path'
import puppeteer from '../frontend/node_modules/puppeteer-core/lib/puppeteer/puppeteer-core.js'

const baseUrl = (process.argv[2] || 'http://127.0.0.1:7311').replace(/\/$/, '')
const artifactDir = path.resolve('artifacts/live-switch-smoke')
await fs.mkdir(artifactDir, { recursive: true })

const browser = await puppeteer.launch({
  executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  headless: true,
  args: ['--enable-webgl', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--no-sandbox'],
})
const page = await browser.newPage()
const errors = []
page.on('console', (message) => {
  const text = message.text()
  const expectedRateLimit = text.includes('status of 429 (Too Many Requests)')
  if (message.type() === 'error' && !expectedRateLimit) errors.push(`console: ${text}`)
})
page.on('pageerror', (error) => errors.push(`page: ${error.message}`))

async function activeSymbol() {
  return page.$eval('.market-heading .eyebrow', (element) => element.textContent.split('·')[0].trim())
}

async function waitForSymbol(symbol, timeoutMs = 90_000) {
  const started = Date.now()
  while (Date.now() - started < timeoutMs) {
    if (await activeSymbol() === symbol) return
    const chart = await page.$('.smile-panel canvas')
    const box = await chart?.boundingBox()
    if (box) {
      const phase = ((Date.now() - started) / 700) % 1
      await page.mouse.move(box.x + box.width * (0.25 + phase * 0.5), box.y + box.height * 0.55)
    }
    await new Promise((resolve) => setTimeout(resolve, 500))
  }
  throw new Error(`timed out waiting for ${symbol}; active=${await activeSymbol()}`)
}

async function requestSymbol(symbol) {
  const input = await page.$('.live-symbol-control input')
  await input.evaluate((element, value) => {
    const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set
    setter.call(element, value)
    element.dispatchEvent(new Event('input', { bubbles: true }))
    element.dispatchEvent(new Event('change', { bubbles: true }))
  }, symbol)
  await page.waitForFunction(
    (target) => document.querySelector('.live-symbol-control input')?.value === target,
    {},
    symbol,
  )
  await page.$eval('.live-run', (button) => button.click())
}

async function assertHealthy(symbol) {
  await page.waitForFunction(
    (target) => document.querySelector('.market-heading .eyebrow')?.textContent?.startsWith(target),
    { timeout: 20_000 },
    symbol,
  )
  await page.waitForFunction(
    () => document.querySelector('.feed-health strong')?.textContent === 'streaming',
    { timeout: 20_000 },
  )
  await page.waitForFunction(
    () => document.querySelector('.svi-toolbar')?.textContent?.includes('SVI ready'),
    { timeout: 20_000 },
  )
}

try {
  await page.setViewport({ width: 1600, height: 1000, deviceScaleFactor: 1 })
  await page.goto(`${baseUrl}/?mode=live`, { waitUntil: 'networkidle2' })
  await page.waitForFunction(
    () => !document.querySelector('.market-heading .eyebrow')?.textContent?.startsWith('--'),
    { timeout: 20_000 },
  )
  const initialSymbol = await activeSymbol()
  const targetSymbol = initialSymbol === 'SPY' ? 'NVDA' : 'SPY'
  await assertHealthy(initialSymbol)
  const initialVersion = await page.$eval('.market-chart', (element) => Number(element.dataset.optionVersion))

  await requestSymbol(targetSymbol)
  await new Promise((resolve) => setTimeout(resolve, 2_000))
  const firstPendingSymbol = await activeSymbol()
  const queuedForward = (await page.$eval('.live-run', (element) => element.textContent)).includes('排队中')
  if (queuedForward && firstPendingSymbol !== initialSymbol) {
    throw new Error(`queued ${targetSymbol} switch replaced the active ${firstPendingSymbol} stream`)
  }
  if (queuedForward) {
    await page.waitForFunction(
      (version) => Number(document.querySelector('.market-chart')?.dataset.optionVersion) > version,
      { timeout: 12_000 },
      initialVersion,
    )
  }
  await waitForSymbol(targetSymbol)
  await assertHealthy(targetSymbol)
  await page.screenshot({ path: path.join(artifactDir, `${targetSymbol.toLowerCase()}-live.png`) })

  const targetVersion = await page.$eval('.market-chart', (element) => Number(element.dataset.optionVersion))
  await requestSymbol(initialSymbol)
  await new Promise((resolve) => setTimeout(resolve, 2_000))
  const secondPendingSymbol = await activeSymbol()
  const queuedReturn = (await page.$eval('.live-run', (element) => element.textContent)).includes('排队中')
  if (queuedReturn && secondPendingSymbol !== targetSymbol) {
    throw new Error(`queued ${initialSymbol} switch replaced the active ${secondPendingSymbol} stream`)
  }
  if (queuedReturn) {
    await page.waitForFunction(
      (version) => Number(document.querySelector('.market-chart')?.dataset.optionVersion) > version,
      { timeout: 12_000 },
      targetVersion,
    )
  }
  await waitForSymbol(initialSymbol)
  await assertHealthy(initialSymbol)
  await page.screenshot({ path: path.join(artifactDir, `${initialSymbol.toLowerCase()}-return-live.png`) })

  if (errors.length) throw new Error(errors.join('\n'))
  console.log(JSON.stringify({
    ok: true,
    initialSymbol,
    targetSymbol,
    queuedForward,
    queuedReturn,
    finalSymbol: await activeSymbol(),
    artifacts: artifactDir,
  }, null, 2))
} finally {
  await browser.close()
}
