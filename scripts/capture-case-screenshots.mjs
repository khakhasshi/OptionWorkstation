import fs from 'node:fs/promises'
import path from 'node:path'
import puppeteer from '../frontend/node_modules/puppeteer-core/lib/puppeteer/puppeteer-core.js'

const ROOT = path.resolve(new URL('..', import.meta.url).pathname)
const OUTPUT = path.join(ROOT, 'docs', 'assets', 'case-studies')
const APP_URL = process.env.OPTION_WORKSTATION_URL || 'http://127.0.0.1:7311/?mode=replay'
const CHROME = process.env.CHROME_BIN || '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome'

const cases = [
  { id: '01-spy-low-vol-positive-gamma', symbol: 'SPY', date: '2026-07-10', minute: '10:00' },
  { id: '02-spy-negative-gamma-move', symbol: 'SPY', date: '2026-06-05', minute: '10:30' },
  { id: '03-amzn-quality-gate', symbol: 'AMZN', date: '2026-04-29', minute: '14:00' },
  { id: '04-spy-high-vrp', symbol: 'SPY', date: '2026-06-17', minute: '14:00' },
  { id: '05-tsla-skew-quality', symbol: 'TSLA', date: '2026-05-13', minute: '15:00' },
]

const delay = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const frameFor = (minute) => {
  const [hours, minutes] = minute.split(':').map(Number)
  return (hours * 60 + minutes) - (9 * 60 + 30)
}

await fs.mkdir(OUTPUT, { recursive: true })
const browser = await puppeteer.launch({
  headless: 'new',
  executablePath: CHROME,
  args: ['--no-sandbox', '--disable-gpu', '--disable-dev-shm-usage'],
})

try {
  for (const item of cases) {
    const page = await browser.newPage()
    await page.setViewport({ width: 1680, height: 1050, deviceScaleFactor: 1 })
    await page.goto(APP_URL, { waitUntil: 'networkidle0', timeout: 30000 })
    await page.waitForSelector('.research-workspace', { timeout: 15000 })
    const symbols = await page.$$eval('button.symbol-chip', (elements) => elements.map((element) => element.textContent.trim()))
    if (!symbols.includes(item.symbol)) {
      await page.select('select:not(.date-select)', item.symbol)
      await delay(500)
    }
    await page.$$eval('button.symbol-chip', (elements, symbol) => {
      const target = elements.find((element) => element.textContent.trim() === symbol)
      target?.click()
    }, item.symbol)
    await delay(500)
    await page.select('select.date-select:not(.compact)', item.date)
    await page.waitForSelector('input[type="range"]', { timeout: 15000 })
    await delay(500)
    await page.$$eval('select:not(.date-select)', (elements, date) => {
      const target = elements.find((element) => [...element.options].some((option) => option.value === date))
      if (!target) return
      const setter = Object.getOwnPropertyDescriptor(HTMLSelectElement.prototype, 'value').set
      setter.call(target, date)
      target.dispatchEvent(new Event('change', { bubbles: true }))
    }, item.date)
    await delay(900)
    const frame = frameFor(item.minute)
    await page.$eval('input[type="range"]', (element, value) => {
      const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, 'value').set
      setter.call(element, String(value))
      element.dispatchEvent(new Event('input', { bubbles: true }))
      element.dispatchEvent(new Event('change', { bubbles: true }))
    }, frame)
    for (let attempt = 0; attempt < 30; attempt += 1) {
      const label = await page.$eval('.timeline strong', (element) => element.textContent.trim())
      if (label.startsWith(item.minute)) break
      await delay(100)
    }
    await delay(900)
    await page.screenshot({
      path: path.join(OUTPUT, `${item.id}.png`),
      fullPage: false,
    })
    console.log(`${item.id}: ${item.symbol} ${item.date} ${item.minute} -> ${path.join(OUTPUT, `${item.id}.png`)}`)
    await page.close()
  }
} finally {
  await browser.close()
}
