/**
 * Drives the demo Google's OAuth verification asks for, while the screen is recorded.
 *
 *   HALCYON_DEMO_EMAIL=<a test Gmail address> node tools/demo-video.cjs
 *
 * What the video has to show (src-tauri/oauth/README.md, route B, step 4): a Google account
 * added in Halcyon; Google's consent screen, with the client ID readable in the address bar; and
 * each use of the `https://mail.google.com/` scope -- reading, sending, moving, deleting and
 * permanently deleting mail.
 *
 * Who does what:
 *
 * - **Recording is Windows' Snipping Tool, started and stopped by a person.** A script that
 *   copies the screen in a loop is what spyware does, and Defender blocked the first one as
 *   such (CHANGELOG, 2026-10-03). This tool records nothing.
 * - **The Google sign-in is a person's too**, in their own browser. It needs the account's
 *   password at Google, and granting an app access to an account is that account owner's act.
 * - **This drives Halcyon**: a release build started against a scratch store, with every
 *   per-user path redirected (`LOCALAPPDATA`, `WEBVIEW2_USER_DATA_FOLDER`), driven over the
 *   DevTools protocol, with a caption laid over the window saying what each step shows.
 *
 * **Never remove the account here.** Credentials are filed by address (`halcyon:<email>`), so the
 * scratch store's account shares its token with any installed Halcyon holding the same address,
 * and removing it would purge that token and sign the installed copy out. Signing in again only
 * replaces the token with a newer one, which every copy can use. When the run ends, the app is
 * closed and the scratch store is left for deleting by hand.
 *
 * `%APPDATA%\com.uniki.halcyon\.window-state.json` cannot be redirected, so it is copied aside
 * and put back, as tools/store-screenshots.cjs does.
 */

'use strict'

const { execFileSync, spawn } = require('node:child_process')
const fs = require('node:fs')
const path = require('node:path')

const root = path.resolve(__dirname, '..')
const exe = path.join(root, 'src-tauri', 'target', 'release', 'halcyon.exe')
const scratch = path.join(root, 'src-tauri', 'target', 'demovideo')
const stopFile = path.join(scratch, 'stop')
const statePath = path.join(process.env.APPDATA ?? '', 'com.uniki.halcyon', '.window-state.json')
const stateBackup = path.join(scratch, 'window-state.user.json')
const PORT = 9333

const email = (process.env.HALCYON_DEMO_EMAIL ?? '').trim()
const startDelay = Number(process.env.HALCYON_DEMO_START_DELAY ?? 25) * 1000
const dryRun = process.env.HALCYON_DEMO_DRY_RUN === '1'

const sleep = (ms) => new Promise((resolve) => setTimeout(resolve, ms))
const log = (line) => {
  console.log(`[${new Date().toISOString().slice(11, 19)}] ${line}`)
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

  /**
   * With a time limit. A page whose window is covered -- Halcyon behind the browser during the
   * Google sign-in -- can stop answering until it is shown again, and a take stalled on exactly
   * that. A command that times out fails, and `until` takes a failure as "not yet".
   */
  send(method, params = {}, timeout = 10_000) {
    const id = ++this.next
    this.socket.send(JSON.stringify({ id, method, params }))
    return new Promise((resolve, reject) => {
      const timer = setTimeout(() => {
        this.waiting.delete(id)
        reject(new Error(`${method} got no answer in ${String(timeout / 1000)} s`))
      }, timeout)
      this.waiting.set(id, {
        resolve: (value) => {
          clearTimeout(timer)
          resolve(value)
        },
        reject: (error) => {
          clearTimeout(timer)
          reject(error)
        },
      })
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
      await sleep(250)
    }
    throw new Error(`timed out waiting for ${what}`)
  }

  /** A real mouse click at the middle of whatever the expression returns. */
  async click(elementExpression, what, button = 'left') {
    const at = await this.eval(`(() => {
      const element = ${elementExpression}
      if (!element) return null
      element.scrollIntoView({ block: 'nearest' })
      const r = element.getBoundingClientRect()
      return { x: r.left + r.width / 2, y: r.top + r.height / 2 }
    })()`)
    if (!at) throw new Error(`could not find ${what}`)

    const event = { x: at.x, y: at.y, button, clickCount: 1 }
    await this.send('Input.dispatchMouseEvent', { type: 'mouseMoved', x: at.x, y: at.y })
    await this.send('Input.dispatchMouseEvent', { type: 'mousePressed', ...event })
    await this.send('Input.dispatchMouseEvent', { type: 'mouseReleased', ...event })
  }

  /** Types into whatever has focus, a character at a time, so the video shows it being typed. */
  async type(text, perCharacter = 45) {
    for (const character of text) {
      await this.send('Input.insertText', { text: character })
      await sleep(perCharacter)
    }
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

  /**
   * A caption over the window, saying what the step shows. Laid over the page rather than drawn
   * into the video, because the recording is the Snipping Tool's and nothing here touches it.
   */
  async caption(text) {
    await this.eval(`(() => {
      let element = document.getElementById('halcyon-demo-caption')
      if (!element) {
        element = document.createElement('div')
        element.id = 'halcyon-demo-caption'
        Object.assign(element.style, {
          position: 'fixed', left: '50%', bottom: '28px', transform: 'translateX(-50%)',
          maxWidth: '78%', padding: '12px 22px', borderRadius: '12px', zIndex: '2147483647',
          background: 'rgba(18, 18, 22, 0.9)', color: '#fff', pointerEvents: 'none',
          font: '600 19px/1.4 "Segoe UI Variable Text", "Segoe UI", sans-serif',
          textAlign: 'center', boxShadow: '0 8px 28px rgba(0, 0, 0, 0.35)',
        })
        document.body.appendChild(element)
      }
      element.textContent = ${JSON.stringify(text)}
      element.style.display = ${JSON.stringify(text)} ? 'block' : 'none'
      return true
    })()`)
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

// By text content rather than rendered text: the first take failed here when the window was
// covered by the Snipping Tool's overlay, and the content of a button does not depend on whether
// anything is drawing it.
const buttonWithText = (text) =>
  `[...document.querySelectorAll('button')].find((b) => b.textContent.trim() === ${JSON.stringify(text)})`

const googleTile = `[...document.querySelectorAll('[role="radio"]')].find((r) => r.textContent.trim().startsWith('Google'))`

const rows = `document.querySelectorAll('[role="listbox"][aria-label="Messages"] [role="option"]')`
const rowWith = (text) =>
  `[...${rows}].find((r) => r.textContent.includes(${JSON.stringify(text)}))`

const toolbar = (label) => `document.querySelector('button[aria-label=${JSON.stringify(label)}]')`

/** The input a `<label>` with this text belongs to. */
const fieldLabelled = (text) => `(() => {
  const label = [...document.querySelectorAll('label')].find((l) => l.innerText.trim() === ${JSON.stringify(text)})
  return label ? document.getElementById(label.htmlFor) : null
})()`

/** The account's Bin, which Gmail calls Bin or Trash depending on the account's language. */
// A sidebar row's text content is its name and then its count, run together: "Bin2", "Inbox3".
const bin = `[...document.querySelectorAll('[role="treeitem"]')].find((t) => /^(Bin|Trash)\\d*$/.test(t.textContent.trim()))`

function running() {
  const listed = execFileSync('tasklist', ['/FI', 'IMAGENAME eq halcyon.exe', '/NH'], {
    encoding: 'utf8',
  })
  return listed.toLowerCase().includes('halcyon.exe')
}

/** Maximised, so the recording shows the app and nothing behind it. */
function windowState() {
  const place = { x: 40, y: 40, width: 2200, height: 1300 }
  return {
    main: {
      ...place,
      prev_x: place.x,
      prev_y: place.y,
      maximized: true,
      visible: true,
      decorated: true,
      fullscreen: false,
    },
  }
}

/**
 * The message the demo sends itself, and the only one it touches. Timed, so no two takes share a
 * subject: Gmail keeps every sent message under Sent Mail and groups same-subject mail into one
 * conversation, and an earlier take's copy must not be what a later take finds.
 */
const SUBJECT = `Sent from Halcyon at ${new Date().toLocaleTimeString('en-GB', { hour: '2-digit', minute: '2-digit' })}`

const mailboxNamed = (name) =>
  `[...document.querySelectorAll('[role="treeitem"]')].find((t) => t.textContent.trim().replace(/\\d+$/, '') === ${JSON.stringify(name)})`

/**
 * Each use of the scope, on one message: sending it, reading it, moving it, deleting it, and
 * deleting it permanently. Nothing else in the mailbox is touched. The account a demo signs in to
 * can hold real mail -- the one the first takes used did -- and the first version of this, which
 * deleted "the first message" and then emptied the whole Bin, would have destroyed some of it.
 *
 * `send: false` uses a message already there, for rehearsing against the invented store that
 * tools/store-screenshots.cjs writes, which cannot send.
 */
async function demonstrate(page, { email, subject, send }) {
  if (send) {
    // ---- sending
    await page.caption(
      "Sending: the https://mail.google.com/ scope lets Halcyon send mail through Gmail's SMTP server.",
    )
    await sleep(2500)
    // The toolbar button rather than Ctrl+N: the shortcut handler ignores keys while focus is in
    // a text field or a sheet is closing, and the fourth take lost its Ctrl+N to one of them.
    // Retried, because the fifth take's click, five seconds after the account was added, opened
    // nothing, while the same click on the same store a minute later opened compose in two
    // seconds. A slow window from an earlier click is looked for before clicking again, so the
    // video never shows two.
    let compose = null
    for (let attempt = 1; attempt <= 3 && compose === null; attempt++) {
      compose = await pageWhere((url) => url.includes('compose'), 'compose', 1000).catch(() => null)
      if (compose !== null) break
      await page.click(toolbar('New Message'), 'New Message')
      compose = await pageWhere((url) => url.includes('compose'), 'compose', 10_000).catch(
        () => null,
      )
      if (compose === null) log(`New Message, attempt ${String(attempt)}: no compose window yet`)
    }
    if (compose === null) throw new Error('New Message opened no compose window in three tries')
    await compose.until(
      `!!document.querySelector('[role="textbox"][aria-label="Message body"]')`,
      'the editor',
    )
    await compose.caption('A new message, to the same account.')
    await sleep(1200)
    await compose.click(`document.querySelector('input')`, 'the To field')
    await compose.type(email)
    await compose.press('Tab', 'Tab', 9)
    await sleep(500)
    await compose.click(`document.querySelector('input[placeholder="Subject"]')`, 'the subject')
    await compose.type(subject)
    await compose.click(
      `document.querySelector('[role="textbox"][aria-label="Message body"]')`,
      'the body',
    )
    await compose.type(
      "This message was written and sent in Halcyon, through Gmail's SMTP server.",
      25,
    )
    await sleep(2000)
    await compose.click(buttonWithText('Send'), 'Send')
    compose.close()

    await page.caption(
      'Sent. Halcyon holds a message for a few seconds so it can be taken back, then sends it.',
    )
    await page.until(`!!${rowWith(subject)}`, 'the message to arrive', 180_000)
    await sleep(1500)
  }

  // ---- reading
  await page.caption('Reading: the same scope lets Halcyon read mail over IMAP.')
  await sleep(2000)
  await page.click(rowWith(subject), 'the message')
  await sleep(6000)

  // ---- moving
  await page.caption('Moving: Move to… files the message in another mailbox.')
  await sleep(2500)
  await page.click(toolbar('Move to'), 'Move to')
  await sleep(2500)
  const destination = await page.eval(`(() => {
    // Only somewhere the sidebar shows, so the video can follow the message there. Gmail's All
    // Mail is in the picker and not in the sidebar.
    const shown = new Set([...document.querySelectorAll('[role="treeitem"]')].map((t) => t.textContent.trim().replace(/\\d+$/, '')))
    const choices = [...document.querySelectorAll('[role="dialog"] button')]
    const named = (name) => shown.has(name) ? choices.find((b) => b.innerText.trim().split('\\n')[0] === name) : undefined
    const pick = named('Archive') ?? named('Important') ?? named('Starred') ?? named('All Mail')
    if (!pick) return null
    pick.setAttribute('data-demo-pick', '1')
    return pick.innerText.trim().split('\\n')[0]
  })()`)
  if (!destination) throw new Error('no mailbox to move to')
  await page.click(`document.querySelector('[data-demo-pick="1"]')`, destination)
  await sleep(1200)
  // In case the picker confirms rather than moving on the click.
  if (await page.eval(`!!${buttonWithText('Move')}`)) {
    await page.click(buttonWithText('Move'), 'Move')
  }
  log(`moved to ${String(destination)}`)
  await sleep(2500)

  // Followed there, so the video shows where it went.
  await page.click(mailboxNamed(destination), destination)
  await page.until(`!!${rowWith(subject)}`, `the message in ${String(destination)}`, 30_000)
  await sleep(1500)
  await page.click(rowWith(subject), 'the message')
  await sleep(3000)

  // ---- deleting
  await page.caption('Deleting: the message goes to the Bin.')
  await sleep(2500)
  await page.click(toolbar('Delete'), 'Delete')
  await sleep(3000)
  await page.click(bin, 'the Bin')
  await page.until(`!!${rowWith(subject)}`, 'the message in the Bin', 30_000)
  await sleep(1500)
  await page.click(rowWith(subject), 'the message')
  await sleep(3000)

  // ---- deleting permanently
  await page.caption(
    'Permanently deleting: Shift+Delete removes the message from Gmail for good — the reason Halcyon needs the full https://mail.google.com/ scope.',
  )
  await sleep(4000)
  await page.press('Delete', 'Delete', 46, 8)
  await sleep(1500)
  // In case it asks first.
  const confirm = `[...document.querySelectorAll('[role="dialog"] button, [role="alertdialog"] button')].find((b) => /^Delete/.test(b.textContent.trim()))`
  if (await page.eval(`!!${confirm}`)) {
    await page.click(confirm, 'the confirmation')
  }
  await page.until(`!${rowWith(subject)}`, 'the message to be gone', 30_000)
  log('deleted permanently')
  await sleep(4000)
}

// ------------------------------------------------------------------ the run

async function main() {
  if (!email.includes('@')) {
    throw new Error('set HALCYON_DEMO_EMAIL to the test Gmail address the video signs in with')
  }
  if (!fs.existsSync(exe)) throw new Error(`no build at ${exe}; run tools/make-msix.ps1 first`)
  if (running()) {
    throw new Error(
      'Halcyon is running. Close it first -- it may be holding drafts, so this will not.',
    )
  }

  // A store and a WebView profile of its own, fresh every run.
  fs.rmSync(path.join(scratch, 'local'), { recursive: true, force: true })
  fs.rmSync(path.join(scratch, 'webview'), { recursive: true, force: true })
  fs.rmSync(stopFile, { force: true })
  fs.mkdirSync(scratch, { recursive: true })

  const hadState = fs.existsSync(statePath)
  if (hadState) fs.copyFileSync(statePath, stateBackup)

  let app
  try {
    fs.mkdirSync(path.dirname(statePath), { recursive: true })
    fs.writeFileSync(statePath, JSON.stringify(windowState(), null, 2))

    app = spawn(exe, [], {
      env: {
        ...process.env,
        LOCALAPPDATA: path.join(scratch, 'local'),
        WEBVIEW2_USER_DATA_FOLDER: path.join(scratch, 'webview'),
        WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS: `--remote-debugging-port=${String(PORT)}`,
      },
      stdio: 'ignore',
    })

    const page = await pageWhere(
      (url) => !url.includes('compose') && !url.includes('settings'),
      'main window',
    )
    await page.until(`!!${buttonWithText('Add your account')}`, 'the welcome', 30_000)
    await page.caption(
      'Halcyon Mail, a desktop email client for Windows 11 — a fresh install, with no accounts yet.',
    )
    log(`ready; starting in ${String(startDelay / 1000)} s -- start the recording now`)
    await sleep(startDelay)

    // ---- adding the account
    await page.caption('Adding a Google account: Add your account → Google.')
    await sleep(2500)

    // From wherever the assistant has got to. A click while the recording was being set up can
    // open it early, and the run should carry on from there rather than fail on a missing button.
    await page.until(
      `!!${buttonWithText('Add your account')} || !!${googleTile} || !!${fieldLabelled('Email Address')}`,
      'the welcome or the account assistant',
    )
    if (await page.eval(`!!${buttonWithText('Add your account')}`)) {
      await page.click(buttonWithText('Add your account'), 'Add your account')
      await sleep(1800)
    }
    if (!(await page.eval(`!!${fieldLabelled('Email Address')}`))) {
      await page.until(`!!${googleTile}`, 'the provider tiles')
      await page.click(googleTile, 'the Google tile')
      await sleep(1500)
      await page.click(buttonWithText('Continue'), 'Continue')
    }
    await page.until(`!!${fieldLabelled('Email Address')}`, 'the email field')
    await sleep(1000)

    await page.click(fieldLabelled('Name'), 'the name field')
    await page.press('a', 'KeyA', 65, 2)
    await page.type('Halcyon Demo')
    await sleep(500)
    await page.click(fieldLabelled('Email Address'), 'the email field')
    await page.press('a', 'KeyA', 65, 2)
    await page.type(email)
    await sleep(1200)

    await page.caption(
      "Halcyon opens Google's own sign-in page in the default browser. It never sees the Google password.",
    )
    await sleep(4000)

    // A rehearsal stops here, before anything opens the browser or talks to Google.
    if (dryRun) {
      const enabled = await page.eval(`!${buttonWithText('Continue')}.disabled`)
      log(`dry run: the account form is filled and Continue is ${enabled ? 'enabled' : 'DISABLED'}`)
      return
    }

    await page.click(buttonWithText('Continue'), 'Continue')
    log('the browser has the sign-in -- over to the person at the keyboard')

    // ---- the person signs in at Google, and comes back to Halcyon
    await page.caption('Waiting for Google sign-in in the browser…')
    // Either Halcyon's report, or -- if the person clicked Add Account themselves, as the third
    // take did while this was still waiting on a covered window -- the mailbox already there.
    await page.until(
      `!!${buttonWithText('Add Account')} || ${rows}.length > 0`,
      'the sign-in to finish',
      10 * 60_000,
    )
    log('signed in')
    if (await page.eval(`!!${buttonWithText('Add Account')}`)) {
      await page.caption(
        "Back in Halcyon: it has signed in to Gmail's IMAP and SMTP servers with the token Google issued.",
      )
      await sleep(6000)
      await page.click(buttonWithText('Add Account'), 'Add Account')
    }

    await page.caption('Halcyon downloads the mailbox to this PC.')
    await page.until(`${rows}.length > 0`, 'the Inbox', 120_000)
    await sleep(4000)

    // Not until the person has brought Halcyon back in front of the browser, or the video would
    // show the browser while everything below happened behind it. Three minutes, then on anyway.
    await page
      .until('document.hasFocus()', 'Halcyon to be in front', 3 * 60_000)
      .then(() => {
        log('halcyon is in front')
      })
      .catch(() => {
        log('Halcyon never took focus; carrying on')
      })
    await sleep(2500)

    // The account's own mailboxes in the sidebar mean its first folder sync is done, and the Bin
    // has to be there for the last step anyway.
    await page.until(`!!${bin}`, "the account's mailboxes", 120_000)
    await sleep(2000)

    try {
      await demonstrate(page, { email, subject: SUBJECT, send: true })
    } catch (error) {
      // What the app was showing, so a failed take says why rather than only where.
      const seen = await page
        .eval(
          `({ dialogs: [...document.querySelectorAll('[role="dialog"], [role="alertdialog"]')].map((d) => d.textContent.slice(0, 160)),
              alerts: [...document.querySelectorAll('[role="status"], [role="alert"]')].map((a) => a.textContent.slice(0, 160)).filter(Boolean),
              focused: document.hasFocus() })`,
        )
        .catch(() => 'the page did not answer')
      const pages = await targets()
        .then((list) => list.filter((t) => t.type === 'page').map((t) => t.url))
        .catch(() => [])
      log(`the app at the failure: ${JSON.stringify(seen)} pages ${JSON.stringify(pages)}`)
      throw error
    }

    await page.caption(
      'All of this mail stays on this PC. Halcyon has no server and sends no data anywhere else.',
    )
    await sleep(8000)
    await page.caption('')
    log('done -- stop the recording; create the stop file to close the app')

    const deadline = Date.now() + 20 * 60_000
    while (!fs.existsSync(stopFile) && Date.now() < deadline) await sleep(500)
    page.close()
  } finally {
    if (app) app.kill()
    for (let i = 0; i < 40 && running(); i++) await sleep(250)
    if (hadState) fs.copyFileSync(stateBackup, statePath)
    log('the app is closed and the window state is back')
  }
}

// Run directly, it records a take; required, it lends `demonstrate` to a rehearsal.
if (require.main === module) {
  main().catch((error) => {
    log(`FAILED: ${error instanceof Error ? error.message : String(error)}`)
    process.exitCode = 1
  })
}

module.exports = { demonstrate, pageWhere, PORT }
