/**
 * A local update server, for testing the updater end to end.
 *
 *   node tools/update-server.cjs <bundle-dir> [port]
 *
 * ## Why this exists
 *
 * The updater is the one part of the app that can replace the app. It is also, of everything
 * built for Phase 11, the piece most likely to be shipped without anybody having watched it run:
 * testing it needs two versions, a signature, a server, and an installed copy of the older one,
 * and each of those is a small enough obstacle that "it compiles" starts to look like evidence.
 *
 * It is not evidence. The failure modes here are all silent from the inside — a signature that
 * does not verify, a version comparison that never fires, a `latest.json` whose URL is subtly
 * wrong — and the symptom is an app that simply never updates, which nobody notices until a
 * security fix does not reach anyone.
 *
 * So: this serves a real `latest.json` and a real signed installer over HTTP, on localhost, and
 * the app is built pointing at it. What that exercises is the whole path — fetch, parse, compare
 * versions, download, verify the minisign signature against the public key in the binary, run
 * the installer, relaunch.
 *
 * ## What it deliberately does not do
 *
 * Serve anything it was not given. It maps exactly two paths and 404s everything else, because a
 * test server that will hand out arbitrary files is a thing that gets left running.
 */

const fs = require('node:fs')
const http = require('node:http')
const path = require('node:path')

const bundleDir = process.argv[2]
const port = Number(process.argv[3] ?? 8787)

if (!bundleDir || !fs.existsSync(bundleDir)) {
  console.error(
    `usage: node tools/update-server.cjs <bundle-dir> [port]\n  no such directory: ${String(bundleDir)}`,
  )
  process.exit(1)
}

/** The installer and its detached signature, as Tauri names them. */
function findArtefacts() {
  const files = fs.readdirSync(bundleDir)

  const installer = files.find((name) => name.endsWith('-setup.exe'))
  const signature = files.find((name) => name.endsWith('-setup.exe.sig'))

  if (!installer) throw new Error(`no *-setup.exe in ${bundleDir}`)
  if (!signature) {
    throw new Error(
      `no *-setup.exe.sig in ${bundleDir}. Tauri writes one only when bundle.createUpdaterArtifacts ` +
        `is true AND TAURI_SIGNING_PRIVATE_KEY holds the key ITSELF - not a path to it. ` +
        `TAURI_SIGNING_PRIVATE_KEY_PATH is ignored silently, which produces exactly this: a build ` +
        `that succeeds and an update nobody signed.`,
    )
  }

  return { installer, signature }
}

const { installer, signature } = findArtefacts()

// The version out of the filename: Halcyon_1.0.1_x64-setup.exe.
const version = /_(\d+\.\d+\.\d+)_/.exec(installer)?.[1]
if (!version) throw new Error(`cannot read a version out of ${installer}`)

const manifest = {
  version,
  notes:
    'A test update, served from localhost. If you are seeing this in a real release, something has gone wrong.',
  pub_date: new Date().toISOString(),
  platforms: {
    'windows-x86_64': {
      // The signature is the *contents* of the .sig file, not a URL to it.
      signature: fs.readFileSync(path.join(bundleDir, signature), 'utf8').trim(),
      url: `http://127.0.0.1:${String(port)}/${encodeURIComponent(installer)}`,
    },
  },
}

const server = http.createServer((request, response) => {
  const url = decodeURIComponent((request.url ?? '/').split('?')[0]).replace(/^\//, '')

  if (url === 'latest.json') {
    console.log(`  -> latest.json  (offering ${version})`)
    response.writeHead(200, { 'content-type': 'application/json' })
    response.end(JSON.stringify(manifest, null, 2))
    return
  }

  if (url === installer) {
    const file = path.join(bundleDir, installer)
    console.log(`  -> ${installer}  (${String(fs.statSync(file).size)} bytes)`)
    response.writeHead(200, { 'content-type': 'application/octet-stream' })
    fs.createReadStream(file).pipe(response)
    return
  }

  // Everything else. A test server that serves whatever it is asked for is a liability.
  console.log(`  -> 404 ${url}`)
  response.writeHead(404)
  response.end('not found')
})

server.listen(port, '127.0.0.1', () => {
  console.log(
    `serving ${version} from ${bundleDir}\n` +
      `  http://127.0.0.1:${String(port)}/latest.json\n` +
      `  http://127.0.0.1:${String(port)}/${installer}\n`,
  )
})
