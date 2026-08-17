const { chromium } = require(process.env.PW_PATH)

async function inspect(page) {
  return page.evaluate(() => {
    const overflow = document.documentElement.scrollWidth > document.documentElement.clientWidth
    const clipped = [...document.querySelectorAll('button, code, strong, p')]
      .filter(el => el.clientWidth > 0 && el.scrollWidth > el.clientWidth + 2 && getComputedStyle(el).whiteSpace === 'nowrap')
      .map(el => ({ tag: el.tagName, text: el.textContent.trim().slice(0, 60), client: el.clientWidth, scroll: el.scrollWidth }))
    const buttons = [...document.querySelectorAll('button')]
      .filter(el => el.offsetParent !== null)
      .map(el => ({ text: el.textContent.trim(), width: el.getBoundingClientRect().width, height: el.getBoundingClientRect().height }))
    return { overflow, clipped, buttons, title: document.title }
  })
}

async function main() {
  const adminStatus = await fetch('http://127.0.0.1:43829/api/status').then(r => r.json())
  const phone = adminStatus.devices.find(d => d.name === '手动配对手机')
  if (!phone) throw new Error('visual test phone missing')
  const browser = await chromium.launch({ headless: true, executablePath: 'C:\\Program Files (x86)\\Microsoft\\Edge\\Application\\msedge.exe' })
  const desktop = await browser.newContext({ viewport: { width: 1440, height: 900 }, deviceScaleFactor: 1 })
  const page = await desktop.newPage()
  await page.goto('http://127.0.0.1:43829', { waitUntil: 'networkidle' })
  await page.screenshot({ path: 'E:\\AgentBell\\artifacts\\desktop-overview.png', fullPage: true })
  const desktopOverview = await inspect(page)
  await page.locator('[data-tab="devices"]').click()
  await page.screenshot({ path: 'E:\\AgentBell\\artifacts\\desktop-devices.png', fullPage: true })
  const desktopDevices = await inspect(page)

  const mobile = await browser.newContext({ viewport: { width: 390, height: 844 }, deviceScaleFactor: 2, isMobile: true, hasTouch: true })
  const mobilePage = await mobile.newPage()
  await mobilePage.goto('http://192.168.3.9:43829', { waitUntil: 'domcontentloaded' })
  await mobilePage.evaluate(token => localStorage.setItem('agentbell_token', token), phone.token)
  await mobilePage.reload({ waitUntil: 'networkidle' })
  await mobilePage.screenshot({ path: 'E:\\AgentBell\\artifacts\\mobile-overview.png', fullPage: true })
  const mobileOverview = await inspect(mobilePage)
  await browser.close()
  const result = { desktopOverview, desktopDevices, mobileOverview }
  console.log(JSON.stringify(result, null, 2))
  if (Object.values(result).some(r => r.overflow || r.clipped.length)) process.exit(2)
}

main().catch(error => { console.error(error); process.exit(1) })
