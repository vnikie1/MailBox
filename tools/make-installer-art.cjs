/**
 * Generates the NSIS installer artwork, with no image-library dependency.
 *
 * NSIS wants **BMP**, and only BMP — it has wanted BMP since 1999 and will not take a PNG. The
 * two images are fixed sizes that MUI2 hard-codes:
 *
 *   headerImage   150 x 57   top-right of every page after the welcome
 *   sidebarImage  164 x 314  the panel down the left of the welcome and finish pages
 *
 * Committed as a generator rather than as two binary files, for the same reason `make-icon.cjs`
 * is: a BMP in the repository is a thing nobody can change, and the first time the icon is
 * adjusted the artwork silently stops matching it. This reads the real app icon and composites
 * it, so running the script again is all that "update the artwork" ever means.
 *
 *   node tools/make-installer-art.cjs
 *
 * ## Why there is a PNG decoder in here
 *
 * To composite the icon it has to be read, and the icon is a PNG. Node ships zlib, which is the
 * hard half; the rest is unfiltering five scanline predictors, which is about thirty lines and
 * is exactly specified. Adding an image dependency to the build for one script that runs by
 * hand is the worse trade.
 */

const fs = require('node:fs')
const path = require('node:path')
const zlib = require('node:zlib')

const ICON = path.join(__dirname, '..', 'src-tauri', 'icons', 'icon.png')
const OUT = path.join(__dirname, '..', 'src-tauri', 'installer')

/** The app's own blue, from make-icon.cjs. The sidebar is built around it. */
const BLUE = [0x00, 0x7a, 0xff]

/* ------------------------------------------------------------------ PNG decoding */

/**
 * Decodes an 8-bit RGBA, non-interlaced PNG into raw pixels.
 *
 * Deliberately narrow: it handles the one shape the project's own icon is in, and throws on
 * anything else rather than guessing. A decoder that quietly mishandles a palette or a bit
 * depth it does not support produces artwork that looks like a bug in the installer.
 */
function decodePng(buffer) {
  if (buffer.readUInt32BE(0) !== 0x89504e47) throw new Error('not a PNG')

  let at = 8
  const idat = []
  let width = 0
  let height = 0

  while (at < buffer.length) {
    const length = buffer.readUInt32BE(at)
    const type = buffer.toString('ascii', at + 4, at + 8)
    const data = buffer.subarray(at + 8, at + 8 + length)

    if (type === 'IHDR') {
      width = data.readUInt32BE(0)
      height = data.readUInt32BE(4)
      const [depth, colour, , , interlace] = [data[8], data[9], data[10], data[11], data[12]]
      if (depth !== 8 || colour !== 6 || interlace !== 0) {
        throw new Error(`unsupported PNG: depth ${depth}, colour type ${colour}`)
      }
    } else if (type === 'IDAT') {
      idat.push(data)
    } else if (type === 'IEND') {
      break
    }

    at += 12 + length
  }

  const raw = zlib.inflateSync(Buffer.concat(idat))
  const stride = width * 4
  const pixels = Buffer.alloc(height * stride)

  // Undo the per-scanline filter. Each row is preceded by one byte naming its predictor, and
  // every predictor refers to the row above — so this has to run in order, top to bottom.
  for (let y = 0; y < height; y += 1) {
    const filter = raw[y * (stride + 1)]
    const source = raw.subarray(y * (stride + 1) + 1, y * (stride + 1) + 1 + stride)
    const row = pixels.subarray(y * stride, (y + 1) * stride)
    const above = y > 0 ? pixels.subarray((y - 1) * stride, y * stride) : null

    for (let x = 0; x < stride; x += 1) {
      const left = x >= 4 ? row[x - 4] : 0
      const up = above ? above[x] : 0
      const upLeft = above && x >= 4 ? above[x - 4] : 0
      const value = source[x]

      switch (filter) {
        case 0:
          row[x] = value
          break
        case 1:
          row[x] = (value + left) & 0xff
          break
        case 2:
          row[x] = (value + up) & 0xff
          break
        case 3:
          row[x] = (value + ((left + up) >> 1)) & 0xff
          break
        case 4: {
          // Paeth: pick whichever neighbour the gradient predicts.
          const p = left + up - upLeft
          const dLeft = Math.abs(p - left)
          const dUp = Math.abs(p - up)
          const dUpLeft = Math.abs(p - upLeft)
          const best = dLeft <= dUp && dLeft <= dUpLeft ? left : dUp <= dUpLeft ? up : upLeft
          row[x] = (value + best) & 0xff
          break
        }
        default:
          throw new Error(`unknown PNG filter ${filter}`)
      }
    }
  }

  return { width, height, pixels }
}

/* ------------------------------------------------------------------ resampling */

/**
 * Box-downsamples RGBA to a target size, averaging in premultiplied alpha.
 *
 * Premultiplied because averaging colour and alpha separately fringes: a transparent pixel
 * carries an arbitrary colour, and letting it contribute at full weight pulls a dark halo
 * around anything with a soft edge.
 */
function resize(source, width, height) {
  const out = Buffer.alloc(width * height * 4)
  const scaleX = source.width / width
  const scaleY = source.height / height

  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const x0 = Math.floor(x * scaleX)
      const x1 = Math.max(x0 + 1, Math.floor((x + 1) * scaleX))
      const y0 = Math.floor(y * scaleY)
      const y1 = Math.max(y0 + 1, Math.floor((y + 1) * scaleY))

      let r = 0
      let g = 0
      let b = 0
      let a = 0
      let n = 0

      for (let sy = y0; sy < y1; sy += 1) {
        for (let sx = x0; sx < x1; sx += 1) {
          const at = (sy * source.width + sx) * 4
          const alpha = source.pixels[at + 3] / 255
          r += source.pixels[at] * alpha
          g += source.pixels[at + 1] * alpha
          b += source.pixels[at + 2] * alpha
          a += source.pixels[at + 3]
          n += 1
        }
      }

      const at = (y * width + x) * 4
      const alpha = a / n
      const unpremultiply = alpha > 0 ? 255 / alpha : 0

      out[at] = Math.min(255, Math.round((r / n) * unpremultiply))
      out[at + 1] = Math.min(255, Math.round((g / n) * unpremultiply))
      out[at + 2] = Math.min(255, Math.round((b / n) * unpremultiply))
      out[at + 3] = Math.round(alpha)
    }
  }

  return { width, height, pixels: out }
}

/* ------------------------------------------------------------------ BMP writing */

/**
 * Writes a 24-bit BMP.
 *
 * Bottom-up, BGR, rows padded to a multiple of four bytes — all three are things the format
 * requires and all three are silently wrong-looking if you skip them: an upside-down image, a
 * blue-and-red-swapped one, and a diagonal shear respectively.
 *
 * 24-bit with no alpha, because NSIS composites nothing. The background is baked in here.
 */
function encodeBmp(canvas) {
  const rowBytes = canvas.width * 3
  const padding = (4 - (rowBytes % 4)) % 4
  const stride = rowBytes + padding
  const pixelBytes = stride * canvas.height

  const header = Buffer.alloc(54)
  header.write('BM', 0, 'ascii')
  header.writeUInt32LE(54 + pixelBytes, 2)
  header.writeUInt32LE(54, 10)
  header.writeUInt32LE(40, 14)
  header.writeInt32LE(canvas.width, 18)
  header.writeInt32LE(canvas.height, 22)
  header.writeUInt16LE(1, 26)
  header.writeUInt16LE(24, 28)
  header.writeUInt32LE(pixelBytes, 34)
  header.writeInt32LE(2835, 38)
  header.writeInt32LE(2835, 42)

  const body = Buffer.alloc(pixelBytes)

  for (let y = 0; y < canvas.height; y += 1) {
    const flipped = canvas.height - 1 - y
    for (let x = 0; x < canvas.width; x += 1) {
      const from = (flipped * canvas.width + x) * 3
      const to = y * stride + x * 3
      body[to] = canvas.rgb[from + 2]
      body[to + 1] = canvas.rgb[from + 1]
      body[to + 2] = canvas.rgb[from]
    }
  }

  return Buffer.concat([header, body])
}

/* ------------------------------------------------------------------ compositing */

function canvasOf(width, height, fill) {
  const rgb = Buffer.alloc(width * height * 3)
  for (let i = 0; i < width * height; i += 1) {
    rgb[i * 3] = fill(i % width, Math.floor(i / width))[0]
    rgb[i * 3 + 1] = fill(i % width, Math.floor(i / width))[1]
    rgb[i * 3 + 2] = fill(i % width, Math.floor(i / width))[2]
  }
  return { width, height, rgb }
}

/** Alpha-blends an RGBA image onto an opaque canvas. */
function draw(canvas, image, left, top) {
  for (let y = 0; y < image.height; y += 1) {
    for (let x = 0; x < image.width; x += 1) {
      const cx = left + x
      const cy = top + y
      if (cx < 0 || cy < 0 || cx >= canvas.width || cy >= canvas.height) continue

      const from = (y * image.width + x) * 4
      const alpha = image.pixels[from + 3] / 255
      if (alpha === 0) continue

      const to = (cy * canvas.width + cx) * 3
      for (let channel = 0; channel < 3; channel += 1) {
        canvas.rgb[to + channel] = Math.round(
          image.pixels[from + channel] * alpha + canvas.rgb[to + channel] * (1 - alpha),
        )
      }
    }
  }
}

/* ------------------------------------------------------------------ the artwork */

const icon = decodePng(fs.readFileSync(ICON))

fs.mkdirSync(OUT, { recursive: true })

// Header: white, because MUI2's header band is white and a coloured tile would read as a
// mismatched rectangle rather than as artwork. Icon right-aligned, which is where MUI2 puts it.
const header = canvasOf(150, 57, () => [0xff, 0xff, 0xff])
draw(header, resize(icon, 40, 40), 150 - 40 - 9, 9)
fs.writeFileSync(path.join(OUT, 'header.bmp'), encodeBmp(header))

// Sidebar: a near-white panel, cooling very slightly downward, with the icon high on it.
//
// The first version washed the panel in the app's own blue, and the icon — a blue rounded
// square — all but disappeared into it. Rendering it and looking is what caught that; the
// dimensions and the file format had all been correct. A light ground also suits the
// application better: its whole argument is that it does not shout.
const sidebar = canvasOf(164, 314, (_x, y) => {
  const t = y / 313
  return [Math.round(0xfb - t * 14), Math.round(0xfc - t * 12), Math.round(0xfe - t * 8)]
})
draw(sidebar, resize(icon, 88, 88), (164 - 88) / 2, 52)

// A hairline of the accent along the bottom edge, so the panel reads as designed rather than
// as a blank area the artwork failed to load into.
for (let x = 0; x < sidebar.width; x += 1) {
  for (let y = sidebar.height - 3; y < sidebar.height; y += 1) {
    const at = (y * sidebar.width + x) * 3
    sidebar.rgb[at] = BLUE[0]
    sidebar.rgb[at + 1] = BLUE[1]
    sidebar.rgb[at + 2] = BLUE[2]
  }
}
fs.writeFileSync(path.join(OUT, 'sidebar.bmp'), encodeBmp(sidebar))

console.log(`wrote header.bmp (150x57) and sidebar.bmp (164x314) to ${OUT}`)
