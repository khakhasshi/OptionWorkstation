import fs from 'node:fs/promises'
import path from 'node:path'
import puppeteer from '../frontend/node_modules/puppeteer-core/lib/puppeteer/puppeteer-core.js'

const baseUrl = (process.argv[2] || 'http://127.0.0.1:7311').replace(/\/$/, '')
const artifactDir = path.resolve('artifacts/guide-smoke')
await fs.mkdir(artifactDir, { recursive: true })

const browser = await puppeteer.launch({
  executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  headless: true,
  args: ['--no-sandbox'],
})
const page = await browser.newPage()
const errors = []
page.on('console', (message) => {
  if (message.type() === 'error') errors.push(`console: ${message.text()}`)
})
page.on('pageerror', (error) => errors.push(`page: ${error.message}`))

async function setInput(selector, value) {
  await page.$eval(selector, (element, nextValue) => {
    element.value = nextValue
    element.dispatchEvent(new Event('input', { bubbles: true }))
  }, value)
}

try {
  await page.setViewport({ width: 1440, height: 1000, deviceScaleFactor: 1 })
  await page.goto(`${baseUrl}/guide.html`, { waitUntil: 'networkidle2' })
  await page.waitForSelector('h1')
  await page.waitForFunction(() => {
    const image = document.querySelector('.screen img')
    return image?.complete && image.naturalWidth >= 1600
  })

  const desktopOverflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)
  if (desktopOverflow > 1) throw new Error(`desktop horizontal overflow: ${desktopOverflow}px`)
  const sectionCount = await page.$$eval('main section', (sections) => sections.length)
  if (sectionCount < 10) throw new Error(`guide only rendered ${sectionCount} sections`)
  await page.screenshot({ path: path.join(artifactDir, 'desktop-guide.png') })

  await page.select('#price-view', 'bearish')
  await page.select('#iv-level', 'high')
  await page.select('#move-view', 'smaller')
  await page.select('#horizon', '0d')
  await setInput('#account', '10000')
  await setInput('#risk-pct', '1')
  await setInput('#max-loss', '150')
  await page.waitForFunction(() => document.querySelector('#structures')?.textContent?.includes('Bear Put Spread'))

  const overBudget = await page.$eval('#budget-state', (element) => element.textContent)
  const zeroDWarning = await page.$eval('#rejects', (element) => element.textContent.includes('0DTE'))
  if (overBudget !== '超预算') throw new Error(`unexpected risk state: ${overBudget}`)
  if (!zeroDWarning) throw new Error('0DTE warning is missing')

  await setInput('#max-loss', '50')
  await page.waitForFunction(() => document.querySelector('#max-quantity')?.textContent === '2')
  const withinBudget = await page.$eval('#budget-state', (element) => element.textContent)
  if (withinBudget !== '预算内') throw new Error(`risk calculator did not update: ${withinBudget}`)
  await page.$eval('#lab', (element) => {
    document.documentElement.style.scrollBehavior = 'auto'
    window.scrollTo(0, element.getBoundingClientRect().top + window.scrollY - 64)
  })
  await new Promise((resolve) => setTimeout(resolve, 200))
  await page.screenshot({ path: path.join(artifactDir, 'decision-lab.png') })

  await page.goto(`${baseUrl}/?mode=replay`, { waitUntil: 'networkidle2' })
  await page.waitForSelector('a.icon-button[href="/guide.html"]', { timeout: 20_000 })

  await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 })
  await page.goto(`${baseUrl}/guide.html`, { waitUntil: 'networkidle2' })
  await page.waitForSelector('h1')
  const mobileOverflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)
  if (mobileOverflow > 1) throw new Error(`mobile horizontal overflow: ${mobileOverflow}px`)
  await page.screenshot({ path: path.join(artifactDir, 'mobile-guide.png') })

  if (errors.length) throw new Error(errors.join('\n'))
  console.log(JSON.stringify({
    ok: true,
    desktopOverflow,
    mobileOverflow,
    sectionCount,
    overBudget,
    withinBudget,
    artifacts: artifactDir,
  }, null, 2))
} finally {
  await browser.close()
}
