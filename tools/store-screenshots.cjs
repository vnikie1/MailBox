/**
 * The Microsoft Store screenshots, taken from the real app showing invented mail. docs/07 §2.7
 * asks for at least one at 1366 x 768 or larger; this makes five at 3200 x 1800.
 *
 * ## Why it is built this way
 *
 * - **Not the running app.** That shows the user's own inbox, and these are published.
 * - **Not the browser build.** It renders no message bodies ("Message bodies are only available
 *   in the desktop app") and its compose window says composing is unavailable -- the two things a
 *   mail client's screenshots are for.
 * - So `storedemo` writes invented correspondence into a store of its own, and the release build
 *   is launched against it with every per-user path redirected: `LOCALAPPDATA` for the store and
 *   the log, `WEBVIEW2_USER_DATA_FOLDER` for the WebView's profile. It is driven over the DevTools
 *   protocol on a plain WebSocket. Playwright's `connectOverCDP` has hung against this WebView
 *   before, and its `browser.close()` closed the app mid-run.
 * - The one per-user file an environment variable cannot move is the window state in `%APPDATA%`.
 *   It is copied aside first and put back afterwards, whatever happens in between.
 * - Captured from the screen (`shoot-window.ps1`), not from the page, so the shots carry the real
 *   Windows frame and the Mica material behind the sidebar.
 *
 * ## Before running
 *
 * Close Halcyon. It is single-instance, so a second copy would hand over to the user's own window
 * and exit. This stops rather than closing it, because the user's copy may be holding drafts.
 *
 *   node tools/store-screenshots.cjs
 *
 * Builds nothing but the demo store: the app is whatever `tools/make-msix.ps1` left in
 * `target/release`, which is the Store build, so the screenshots show what the package contains.
 * Written to `store/screenshots/`.
 */
const { execFileSync, spawn } = require('node:child_process')
const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const exe = path.join(root, 'src-tauri', 'target', 'release', 'halcyon.exe')
const scratch = path.join(root, 'src-tauri', 'target', 'storeshots')
const local = path.join(scratch, 'local')
const store = path.join(local, 'com.uniki.halcyon', 'halcyon.db')
const out = path.join(root, 'store', 'screenshots')
const appData = process.env.APPDATA ?? ''
const statePath = path.join(appData, 'com.uniki.halcyon', '.window-state.json')
const stateBackup = path.join(scratch, 'window-state.user.json')
const PORT = 9333

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))

/** The `modifiers` bit for Ctrl in `Input.dispatchKeyEvent`. */
const CTRL = 2

/**
 * Transparency for the light shots: opaque, the app's own Reduce Transparency setting.
 *
 * The window's material follows the Windows theme, not the app's. On the machine these were first
 * taken on, Windows is set to dark, and the light theme's sidebar sat on dark Mica and came out a
 * dull mid-grey (#AEB1B9) that no light-mode user would ever see. Opaque surfaces give the light
 * theme as a light-mode machine shows it, without changing anybody's Windows theme to get it.
 * `HALCYON_SHOTS_LIGHT_TRANSPARENCY=system` restores the material, for a machine set to light.
 */
const LIGHT_TRANSPARENCY = process.env.HALCYON_SHOTS_LIGHT_TRANSPARENCY ?? 'reduce'

/**
 * Moves the real pointer to the bottom-right corner of the screen. The protocol's mouse is a
 * separate thing: the actual cursor resting over the window would leave whatever it is over
 * drawn in its hover state in every shot.
 */
function parkPointer() {
  execFileSync(
    'powershell.exe',
    [
      '-NoProfile',
      '-Command',
      'Add-Type -AssemblyName System.Windows.Forms; ' +
        '$b = [System.Windows.Forms.Screen]::PrimaryScreen.Bounds; ' +
        '[System.Windows.Forms.Cursor]::Position = New-Object System.Drawing.Point(($b.Right - 2), ($b.Bottom - 2))',
    ],
    { stdio: 'ignore' },
  )
}

// ------------------------------------------------------------------ the DevTools protocol

class Page {
  static async open(url) {
    const socket = new WebSocket(url)
    await new Promise((resolve, reject) => {
      socket.onopen = resolve
      socket.onerror = () => {
        reject(new Error(`could not open ${url}`))
      }
    })
    return new Page(socket)
  }

  constructor(socket) {
    this.socket = socket
    this.next = 0
    this.waiting = new Map()
    socket.onmessage = (event) => {
      const message = JSON.parse(event.data)
      const pending = this.waiting.get(message.id)
      if (!pending) return
      this.waiting.delete(message.id)
      if (message.error) pending.reject(new Error(message.error.message))
      else pending.resolve(message.result)
    }
  }

  send(method, params = {}) {
    const id = ++this.next
    this.socket.send(JSON.stringify({ id, method, params }))
    return new Promise((resolve, reject) => {
      this.waiting.set(id, { resolve, reject })
    })
  }

  async eval(expression) {
    const result = await this.send('Runtime.evaluate', {
      expression,
      awaitPromise: true,
      returnByValue: true,
    })
    if (result.exceptionDetails) {
      const detail = result.exceptionDetails.exception?.description ?? result.exceptionDetails.text
      throw new Error(detail)
    }
    return result.result.value
  }

  async until(expression, what, timeout = 15_000) {
    const deadline = Date.now() + timeout
    while (Date.now() < deadline) {
      if (await this.eval(expression).catch(() => false)) return
      await sleep(200)
    }
    throw new Error(`timed out waiting for ${what}`)
  }

  /** A real mouse click at the middle of whatever the expression returns. */
  async click(elementExpression, what) {
    const box = await this.eval(`(() => {
      const element = ${elementExpression}
      if (!element) return null
      element.scrollIntoView({ block: 'nearest' })
      const r = element.getBoundingClientRect()
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 }
    })()`)
    if (!box) throw new Error(`could not find ${what}`)

    const at = { x: box.x, y: box.y, button: 'left', clickCount: 1 }
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: at.x, y: at.y })
    await this.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...at })
    await this.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...at })
  }

  /** Moves the pointer off the content, so no row is left drawn in its hover state. */
  async rest() {
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: 1, y: 1 })
  }

  async press(key, code, keyCode, modifiers = 0) {
    for (const type of ['keyDown', 'keyUp']) {
      await this.send('Input.dispatchKeyEvent', {
        type,
        key,
        code,
        windowsVirtualKeyCode: keyCode,
        modifiers,
      })
    }
  }

  close() {
    this.socket.close()
  }
}

async function targets() {
  const response = await fetch(`http://127.0.0.1:${String(PORT)}/json/list`)
  return response.json()
}

async function pageWhere(predicate, what, timeout = 30_000) {
  const deadline = Date.now() + timeout
  while (Date.now() < deadline) {
    const found = await targets()
      .then((list) => list.find((t) => t.type === 'page' && predicate(t.url)))
      .catch(() => undefined)
    if (found) return Page.open(found.webSocketDebuggerUrl)
    await sleep(300)
  }
  throw new Error(`no ${what} appeared`)
}

// ------------------------------------------------------------------ the app

const row = (index) =>
  `document.querySelectorAll('[role="listbox"][aria-label="Messages"] [role="option"]')[${String(index)}]`

const rowFrom = (sender) =>
  `[...document.querySelectorAll('[role="listbox"][aria-label="Messages"] [role="option"]')]
     .find((r) => r.textContent.includes(${JSON.stringify(sender)}))`

const mailbox = (name) =>
  `[...document.querySelectorAll('[role="treeitem"]')]
     .find((item) => item.textContent.trim().startsWith(${JSON.stringify(name)}))`

const button = (label) => `document.querySelector('button[aria-label=${JSON.stringify(label)}]')`

/**
 * Sets the theme where the app's own picker does -- the database, which is trusted on the next
 * load, and the first-frame cache -- then reloads.
 *
 * The reload is a protocol command rather than `location.reload()` inside the evaluation: a page
 * that navigates while an evaluation is pending can take the reply down with it.
 *
 * Blue is Mail's own accent. A user gets their Windows accent unless they pick one, so this is
 * the one place the screenshots choose something for them, and it is the colour the app is
 * modelled on.
 */
async function display(page, theme, transparency = 'system') {
  const value = JSON.stringify({ theme, density: 'default', transparency, accent: 'blue' })
  await page.eval(`(async () => {
    await window.__TAURI_INTERNALS__.invoke('display_preferences_set', { value: ${JSON.stringify(value)} })
    localStorage.setItem('halcyon.settings.display', JSON.stringify({ state: ${value}, version: 0 }))
    return true
  })()`)
  await page.send('Page.reload')
  await sleep(1500)
  await ready(page)
}

async function ready(page) {
  await page.until(
    `!!document.querySelector('[role="listbox"][aria-label="Messages"] [role="option"]')`,
    'the message list',
  )
  await page.eval('document.fonts.ready.then(() => true)')
}

/** Opens a message and gives its body time to render in the reader's frame. */
async function open(page, rowExpression, what) {
  await page.click(rowExpression, what)
  await sleep(1800)
  await page.rest()
  await sleep(400)
}

/**
 * The window's client area, without Windows' caption strip. The caption follows the Windows theme
 * and not the app's, so on a machine set to dark the light shots would carry a dark bar across
 * the top; without it, light and dark match.
 */
function shoot(name, { noActivate = false } = {}) {
  const file = path.join(out, `${name}.png`)
  const args = ['-NoProfile', '-ExecutionPolicy', 'Bypass', '-File']
  args.push(path.join(root, 'tools', 'shoot-window.ps1'), '-Out', file, '-Title', 'Halcyon')
  args.push('-ClientOnly')
  if (noActivate) args.push('-NoActivate')
  const said = execFileSync('powershell.exe', args, { encoding: 'utf8' }).trim()
  console.log(`  ${said}`)
}

// ------------------------------------------------------------------ the run

function running() {
  const listed = execFileSync('tasklist', ['/FI', 'IMAGENAME eq halcyon.exe', '/NH'], {
    encoding: 'utf8',
  })
  return listed.toLowerCase().includes('halcyon.exe')
}

/**
 * Where the window opens, in physical pixels. The saved size is the client area, which is what
 * `shoot` captures, so the shots come out at exactly 3200 x 1800 -- 16:9, the Store's own
 * proportion, and inside its 3840 x 2160 ceiling.
 */
function windowState() {
  const main = { x: 48, y: 40, width: 3200, height: 1800 }
  const entry = (place) => ({
    ...place,
    prev_x: place.x,
    prev_y: place.y,
    maximized: false,
    visible: true,
    decorated: true,
    fullscreen: false,
  })

  return {
    main: entry(main),
    // Over the lower right of the main window, so the compose shot shows both.
    'compose-1': entry({ x: main.x + 1560, y: main.y + 400, width: 1520, height: 1240 }),
  }
}

async function main() {
  if (!fs.existsSync(exe)) throw new Error(`no build at ${exe}; run tools/make-msix.ps1 first`)
  if (running()) {
    throw new Error(
      'Halcyon is running. Close it first -- it may be holding drafts, so this will not.',
    )
  }

  fs.mkdirSync(out, { recursive: true })
  fs.mkdirSync(scratch, { recursive: true })

  console.log('writing the demo store')
  execFileSync(
    'cargo',
    [
      'run',
      '--quiet',
      '--manifest-path',
      path.join(root, 'src-tauri', 'Cargo.toml'),
      '--features',
      'devtools',
      '--bin',
      'storedemo',
      '--',
      '--path',
      store,
      '--reset',
    ],
    { cwd: root, stdio: 'inherit' },
  )

  // A fresh WebView profile, so nothing remembered from an earlier run picks the mailbox.
  fs.rmSync(path.join(scratch, 'webview'), { recursive: true, force: true })

  const hadState = fs.existsSync(statePath)
  if (hadState) fs.copyFileSync(statePath, stateBackup)

  let app
  try {
    fs.mkdirSync(path.dirname(statePath), { recursive: true })
    fs.writeFileSync(statePath, JSON.stringify(windowState(), null, 2))

    console.log('starting the Store build against it')
    app = spawn(exe, [], {
      env: {
        ...process.env,
        LOCALAPPDATA: local,
        WEBVIEW2_USER_DATA_FOLDER: path.join(scratch, 'webview'),
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${String(PORT)}`,
      },
      stdio: 'ignore',
    })

    const page = await pageWhere(
      (url) => !url.includes('compose') && !url.includes('settings'),
      'main window',
    )
    await ready(page)
    parkPointer()

    // Light first and compose last. A compose window stays open across a reload, so anything
    // after it would be shot with a half-written reply floating over it.
    console.log('light')
    await display(page, 'light', LIGHT_TRANSPARENCY)
    await page.click(mailbox('All Inboxes'), 'All Inboxes')
    await sleep(800)
    await open(page, row(0), 'the newest message')
    shoot('02-conversation-light')

    await page.click(`document.querySelector('input[placeholder="Search"]')`, 'the search field')
    await page.send('Input.insertText', { text: 'Lisbon' })
    await page.press('Enter', 'Enter', 13)
    await sleep(1500)
    await open(page, rowFrom('Northwind Air'), 'the booking')
    shoot('05-search-light')

    console.log('dark')
    await display(page, 'dark')
    await page.click(mailbox('All Inboxes'), 'All Inboxes')
    await sleep(800)
    await open(page, row(0), 'the newest message')
    shoot('01-conversation-dark')

    await open(page, rowFrom('The Weekly Brief'), 'the newsletter')
    shoot('03-newsletter-dark')

    console.log('compose')
    await open(page, row(0), 'the newest message')
    await page.click(button('Reply'), 'the Reply button')
    const compose = await pageWhere((url) => url.includes('compose'), 'compose window')
    await compose.until(`!!document.querySelector('[contenteditable="true"]')`, 'the editor')
    await sleep(1200)
    // Into the editor, then to its very top: a reply opens with the quote below an empty line,
    // and a click lands wherever the middle of the editor happens to be.
    await compose.click(`document.querySelector('[contenteditable="true"]')`, 'the editor')
    await compose.press('Home', 'Home', 36, CTRL)
    await compose.send('Input.insertText', {
      text: 'Thanks Jonas — both changes are done. The poster headline now clears the fold, and the legal line is 8pt throughout.',
    })
    await compose.press('Enter', 'Enter', 13)
    await compose.press('Enter', 'Enter', 13)
    await compose.send('Input.insertText', { text: 'Final files to follow before lunch.' })
    await compose.rest()
    await sleep(800)
    shoot('04-compose-dark', { noActivate: true })

    compose.close()
    page.close()
  } finally {
    if (app) {
      // Ended rather than closed: the demo copy is disposable, and closing would put a "save
      // this draft?" question in front of a run with nobody to answer it.
      try {
        execFileSync('taskkill', ['/PID', String(app.pid), '/T', '/F'], { stdio: 'ignore' })
      } catch {
        // Already gone.
      }
      await sleep(1000)
    }

    if (hadState) fs.copyFileSync(stateBackup, statePath)
    else fs.rmSync(statePath, { force: true })
    console.log('the window state is back as it was')
  }

  console.log(`\nscreenshots in ${out}`)
}

main().catch((error) => {
  console.error(error)
  process.exitCode = 1
})
