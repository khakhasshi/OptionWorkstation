import fs from 'node:fs/promises'
import path from 'node:path'
import puppeteer from '../frontend/node_modules/puppeteer-core/lib/puppeteer/puppeteer-core.js'

const baseUrl = (process.argv[2] || 'http://127.0.0.1:7311').replace(/\/$/, '')
const artifactDir = path.resolve('artifacts/thetadata-live-browser-smoke')
await fs.mkdir(artifactDir, { recursive: true })

const browser = await puppeteer.launch({
  executablePath: '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
  headless: true,
  args: ['--enable-webgl', '--enable-unsafe-swiftshader', '--ignore-gpu-blocklist', '--no-sandbox'],
})
const page = await browser.newPage()
const errors = []
page.on('console', (message) => {
  if (message.type() === 'error') errors.push(`console: ${message.text()}`)
})
page.on('pageerror', (error) => errors.push(`page: ${error.message}`))

async function selectThetaData() {
  await page.waitForSelector('.provider-switch', { timeout: 20_000 })
  await page.$$eval('.provider-switch button', (buttons) => {
    const button = buttons.find((item) => item.textContent?.trim() === 'ThetaData')
    if (!button) throw new Error('ThetaData provider button is missing')
    button.click()
  })
  await page.waitForFunction(
    () => document.querySelector('.market-heading .eyebrow')?.textContent?.includes('THETADATA POLL'),
    { timeout: 20_000 },
  )
}

try {
  await page.setViewport({ width: 1600, height: 1000, deviceScaleFactor: 1 })
  await page.goto(`${baseUrl}/?mode=live`, { waitUntil: 'networkidle2' })
  await page.waitForSelector('.research-workspace', { timeout: 20_000 })
  await selectThetaData()
  await page.waitForFunction(
    () => document.querySelector('.feed-health strong')?.textContent === 'streaming',
    { timeout: 30_000 },
  )
  await page.waitForFunction(
    () => document.querySelector('.snapshot-panel small')?.textContent?.includes('ThetaData'),
    { timeout: 30_000 },
  )

  const initialSequence = await page.evaluate(async () => {
    const response = await fetch('/api/live/snapshot?provider=thetadata')
    return (await response.json()).sequence
  })
  await page.waitForFunction(
    async (sequence) => {
      const response = await fetch('/api/live/snapshot?provider=thetadata')
      return (await response.json()).sequence > sequence
    },
    { timeout: 30_000 },
    initialSequence,
  )
  const nextSequence = await page.evaluate(async () => {
    const response = await fetch('/api/live/snapshot?provider=thetadata')
    return (await response.json()).sequence
  })
  if (!(nextSequence > initialSequence)) {
    throw new Error(`ThetaData polling sequence did not advance: ${initialSequence} -> ${nextSequence}`)
  }

  const expirationState = await page.$eval('.snapshot-panel .panel-title select', (select) => ({
    current: select.value,
    options: Array.from(select.options, (option) => option.value),
  }))
  const targetExpiration = expirationState.options.find((item) => item !== expirationState.current)
  if (!targetExpiration) {
    throw new Error(`ThetaData live session needs at least two expirations, found: ${expirationState.options.join(', ')}`)
  }

  const waitForExpiration = async (expectedExpiration) => {
    await page.waitForFunction(
      async (expected) => {
        const select = document.querySelector('.snapshot-panel .panel-title select')
        if (select?.value !== expected) return false
        const response = await fetch('/api/live/snapshot?provider=thetadata')
        if (!response.ok) return false
        const snapshot = await response.json()
        return snapshot.feed?.expiration === expected && snapshot.chain?.expiration === expected
      },
      { timeout: 60_000 },
      expectedExpiration,
    )
  }

  await page.select('.snapshot-panel .panel-title select', targetExpiration)
  await waitForExpiration(targetExpiration)
  const switchedSequence = await page.evaluate(async () => {
    const response = await fetch('/api/live/snapshot?provider=thetadata')
    return (await response.json()).sequence
  })
  await page.waitForFunction(
    async ({ expectedExpiration, sequence }) => {
      const response = await fetch('/api/live/snapshot?provider=thetadata')
      if (!response.ok) return false
      const snapshot = await response.json()
      return snapshot.sequence > sequence && snapshot.feed?.expiration === expectedExpiration
    },
    { timeout: 30_000 },
    { expectedExpiration: targetExpiration, sequence: switchedSequence },
  )
  const expirationAfterNextFrame = await page.$eval('.snapshot-panel .panel-title select', (select) => select.value)
  if (expirationAfterNextFrame !== targetExpiration) {
    throw new Error(`expiration reverted after a live frame: ${targetExpiration} -> ${expirationAfterNextFrame}`)
  }

  await page.select('.snapshot-panel .panel-title select', expirationState.current)
  await waitForExpiration(expirationState.current)

  const desktopOverflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)
  if (desktopOverflow > 1) throw new Error(`desktop horizontal overflow: ${desktopOverflow}px`)

  await page.$eval('.connection-button', (button) => button.click())
  await page.waitForSelector('.theta-credential-form', { timeout: 10_000 })
  const drawerText = await page.$eval('.credential-drawer', (element) => element.textContent)
  for (const expected of ['ThetaData 快照轮询', '仅驻留进程内存', 'ThetaData 模式不开放券商下单']) {
    if (!drawerText.includes(expected)) throw new Error(`connection drawer is missing: ${expected}`)
  }
  await page.screenshot({ path: path.join(artifactDir, 'desktop-thetadata-live.png') })

  await page.setViewport({ width: 390, height: 844, deviceScaleFactor: 1 })
  await page.reload({ waitUntil: 'networkidle2' })
  await page.waitForSelector('.research-workspace', { timeout: 20_000 })
  await selectThetaData()
  const mobileOverflow = await page.evaluate(() => document.documentElement.scrollWidth - window.innerWidth)
  if (mobileOverflow > 1) throw new Error(`mobile horizontal overflow: ${mobileOverflow}px`)
  await page.screenshot({ path: path.join(artifactDir, 'mobile-thetadata-live.png') })

  if (errors.length) throw new Error(errors.join('\n'))
  console.log(JSON.stringify({
    ok: true,
    desktopOverflow,
    mobileOverflow,
    initialSequence,
    nextSequence,
    initialExpiration: expirationState.current,
    switchedExpiration: targetExpiration,
    expirationAfterNextFrame,
    restoredExpiration: expirationState.current,
    artifacts: artifactDir,
  }, null, 2))
} finally {
  await browser.close()
}
