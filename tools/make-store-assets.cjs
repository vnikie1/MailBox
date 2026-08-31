/**
 * Generates the MSIX asset set from the app icon.
 *
 *   node tools/make-store-assets.cjs
 *
 * ## Why there are so many, and why they are not hand-made
 *
 * Windows picks an icon by *purpose* and then by scale, and it does not resample gracefully: a
 * 150px tile scaled down for a 16px list entry is mud. So the manifest names a handful of base
 * images and Windows resolves `.scale-100`, `.scale-200` and `.targetsize-N` variants beside
 * them by filename convention.
 *
 * docs/07 §2.4 is explicit that this set should be generated from one source rather than
 * hand-produced, and the reason is not effort. Forty files made by hand are forty files that
 * drift: the icon changes, thirty-nine are updated, and the one that is not shows up months
 * later on somebody's taskbar.
 *
 * ## altform-unplated
 *
 * Windows normally draws a small icon on a coloured plate. On the taskbar and in the Start
 * jump list it does not, and if no unplated variant exists it uses the plated one — which is
 * the icon with a background baked in, on a background. That is the "why does my taskbar icon
 * have a box around it" bug, and these files are the fix.
 */

const fs = require('node:fs')
const path = require('node:path')

const { decodePng, resize, encodePng } = require('./png.cjs')

const ICON = path.join(__dirname, '..', 'src-tauri', 'icons', 'icon.png')
const OUT = path.join(__dirname, '..', 'src-tauri', 'msix', 'Assets')

/**
 * The base images the manifest names, with the scales Windows asks for.
 *
 * Scale factors are display scaling: 100% is a 1x screen, 400% is a 4K laptop panel. Windows
 * will fall back to a smaller one, so a missing 400 is blurry rather than broken — but blurry
 * on the machines most likely to be running a new Windows 11 install.
 */
const TILES = [
  { name: 'Square44x44Logo', size: 44, scales: [100, 125, 150, 200, 400] },
  { name: 'Square71x71Logo', size: 71, scales: [100, 125, 150, 200, 400] },
  { name: 'Square150x150Logo', size: 150, scales: [100, 125, 150, 200, 400] },
  { name: 'Square310x310Logo', size: 310, scales: [100, 125, 150, 200, 400] },
  { name: 'StoreLogo', size: 50, scales: [100, 125, 150, 200, 400] },
]

/**
 * Sizes the app list, taskbar and Alt-Tab ask for by pixel count rather than by scale.
 *
 * Each is emitted twice: plated, and `altform-unplated` for the surfaces that draw no plate.
 */
const TARGET_SIZES = [16, 24, 32, 48, 256]

const icon = decodePng(fs.readFileSync(ICON))

fs.rmSync(OUT, { recursive: true, force: true })
fs.mkdirSync(OUT, { recursive: true })

let written = 0

function write(name, size) {
  fs.writeFileSync(path.join(OUT, name), encodePng(resize(icon, size, size)))
  written += 1
}

for (const tile of TILES) {
  for (const scale of tile.scales) {
    const size = Math.round((tile.size * scale) / 100)
    write(`${tile.name}.scale-${String(scale)}.png`, size)
  }
}

for (const size of TARGET_SIZES) {
  write(`Square44x44Logo.targetsize-${String(size)}.png`, size)
  // The same image without a plate. See the module note.
  write(`Square44x44Logo.targetsize-${String(size)}_altform-unplated.png`, size)
}

// The wide tile, which is the one shape that is not square. Rather than stretch the icon —
// which would be visibly wrong — it is centred on a transparent field at the right height.
const WIDE = [{ scale: 100, width: 310, height: 150 }]
for (const { scale, width, height } of WIDE) {
  const glyph = resize(icon, height - 30, height - 30)
  const canvas = Buffer.alloc(width * height * 4)
  const left = Math.round((width - glyph.width) / 2)
  const top = Math.round((height - glyph.height) / 2)

  for (let y = 0; y < glyph.height; y += 1) {
    glyph.pixels.copy(
      canvas,
      ((top + y) * width + left) * 4,
      y * glyph.width * 4,
      (y + 1) * glyph.width * 4,
    )
  }

  fs.writeFileSync(
    path.join(OUT, `Wide310x150Logo.scale-${String(scale)}.png`),
    encodePng({ width, height, pixels: canvas }),
  )
  written += 1
}

console.log(`wrote ${String(written)} MSIX assets to ${OUT}`)
