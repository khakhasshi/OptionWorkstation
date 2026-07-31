import fs from 'node:fs/promises'
import path from 'node:path'
import puppeteer from '../frontend/node_modules/puppeteer-core/lib/puppeteer/puppeteer-core.js'

const baseUrl = (process.argv[2] || 'http://127.0.0.1:7311').replace(/\/$/, '')
const artifactDir = path.resolve('artifacts/assistant-smoke')
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

async function openAssistant() {
  await page.waitForSelector('.assistant-launcher', { timeout: 20_000 })
  await page.$eval('.assistant-launcher', (button) => button.click())
  await page.waitForSelector('.assistant-dock', { timeout: 20_000 })
}

async function assertAssistantInsideViewport() {
  const result = await page.$eval('.assistant-dock', (element) => {
    const rect = element.getBoundingClientRect()
    return {
      left: rect.left,
      top: rect.top,
      right: rect.right,
      bottom: rect.bottom,
      viewportWidth: window.innerWidth,
      viewportHeight: window.innerHeight,
      overflow: document.documentElement.scrollWidth - window.innerWidth,
    }
  })
  if (result.left < -1 || result.top < -1 || result.right > result.viewportWidth + 1 || result.bottom > result.viewportHeight + 1) {
    throw new Error(`assistant is outside viewport: ${JSON.stringify(result)}`)
  }
  if (result.overflow > 1) throw new Error(`assistant horizontal overflow: ${result.overflow}px`)
  return result
}

try {
  await page.setViewport({ width: 1600, height: 1000, deviceScaleFactor: 1 })
  await page.goto(`${baseUrl}/?mode=replay`, { waitUntil: 'networkidle2' })
  await page.waitForSelector('.chain-table .add-leg', { timeout: 30_000 })
  await openAssistant()
  const desktop = await assertAssistantInsideViewport()
  await page.waitForSelector('.assistant-quick button', { timeout: 10_000 })
  await page.$eval('.assistant-quick', (container) => {
    const buttons = container.querySelectorAll('button')
    buttons[0]?.click()
    buttons[1]?.click()
  })
  await page.waitForFunction(
    () => {
      const messages = [...document.querySelectorAll('.assistant-message.assistant > div')]
      return messages.some((node) => {
        const text = node.textContent || ''
        return text.length > 40 && !text.includes('正在冻结') && !text.includes('正在生成')
      })
    },
    { timeout: 90_000 },
  )
  await page.waitForFunction(
    () => document.querySelector('.assistant-quick button')?.disabled === false,
    { timeout: 90_000 },
  )
  const desktopMessageCount = await page.$$eval('.assistant-message', (items) => items.length)
  if (desktopMessageCount !== 2) throw new Error(`expected 2 desktop messages, got ${desktopMessageCount}`)
  await page.screenshot({ path: path.join(artifactDir, 'desktop-assistant.png') })
  await page.$eval('.assistant-header button[title="隐藏助手"]', (button) => button.click())
  await page.waitForSelector('.assistant-launcher', { timeout: 10_000 })

  await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 })
  await page.reload({ waitUntil: 'networkidle2' })
  await page.waitForSelector('.research-workspace', { timeout: 30_000 })
  await openAssistant()
  const mobile = await assertAssistantInsideViewport()
  await page.screenshot({ path: path.join(artifactDir, 'mobile-assistant.png') })

  if (errors.length) throw new Error(errors.join('\n'))
  console.log(JSON.stringify({
    ok: true,
    desktop,
    mobile,
    desktopMessageCount,
    artifacts: artifactDir,
  }, null, 2))
} finally {
  await browser.close()
}
