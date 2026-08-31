/**
 * PNG in, PNG or BMP out, with no image-library dependency.
 *
 * Shared by `make-installer-art.cjs` and `make-store-assets.cjs`, both of which need to read the
 * app icon and write it out at other sizes. Node ships zlib, which is the hard half of PNG; the
 * rest is unfiltering five scanline predictors and a CRC, all exactly specified.
 *
 * Adding an image dependency to the build for two scripts that run by hand is the worse trade —
 * and a build dependency that produces the icons is a build dependency that can silently change
 * them.
 */

const zlib = require('node:zlib')

/* ------------------------------------------------------------------------ decode */

/**
 * Decodes an 8-bit RGBA, non-interlaced PNG into raw pixels.
 *
 * Deliberately narrow: it handles the one shape this project's icons are in, and throws on
 * anything else rather than guessing. A decoder that quietly mishandles a palette or a bit depth
 * produces artwork that looks like a rendering bug somewhere else entirely.
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
      const depth = data[8]
      const colour = data[9]
      const interlace = data[12]
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
  // every predictor refers to the row above — so this must run in order, top to bottom.
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
          // Paeth: whichever neighbour the gradient predicts.
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

/* ---------------------------------------------------------------------- resample */

/**
 * Box-downsamples RGBA to a target size, averaging in premultiplied alpha.
 *
 * Premultiplied because averaging colour and alpha separately fringes: a fully transparent pixel
 * still carries some colour, and letting it contribute at full weight pulls a dark halo around
 * every soft edge. On a 16px app-list icon that halo is most of the icon.
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

/* ------------------------------------------------------------------------ encode */

const CRC_TABLE = (() => {
  const table = new Int32Array(256)
  for (let n = 0; n < 256; n += 1) {
    let c = n
    for (let k = 0; k < 8; k += 1) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    table[n] = c
  }
  return table
})()

function crc32(buffer) {
  let crc = 0xffffffff
  for (const byte of buffer) crc = CRC_TABLE[(crc ^ byte) & 0xff] ^ (crc >>> 8)
  return (crc ^ 0xffffffff) >>> 0
}

function chunk(type, data) {
  const length = Buffer.alloc(4)
  length.writeUInt32BE(data.length)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(body))
  return Buffer.concat([length, body, crc])
}

/** Encodes RGBA pixels as an 8-bit RGBA PNG. */
function encodePng(image) {
  const stride = image.width * 4
  const raw = Buffer.alloc(image.height * (stride + 1))

  for (let y = 0; y < image.height; y += 1) {
    // Filter 0 throughout: these are small images and deflate does the work. A cleverer filter
    // choice would save bytes nobody is counting.
    raw[y * (stride + 1)] = 0
    image.pixels.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride)
  }

  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(image.width, 0)
  ihdr.writeUInt32BE(image.height, 4)
  ihdr[8] = 8
  ihdr[9] = 6

  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

/* --------------------------------------------------------------------------- BMP */

/** A canvas of opaque RGB, for the formats that have no alpha. */
function canvasOf(width, height, fill) {
  const rgb = Buffer.alloc(width * height * 3)
  for (let y = 0; y < height; y += 1) {
    for (let x = 0; x < width; x += 1) {
      const [r, g, b] = fill(x, y)
      const at = (y * width + x) * 3
      rgb[at] = r
      rgb[at + 1] = g
      rgb[at + 2] = b
    }
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

/**
 * Writes a 24-bit BMP.
 *
 * Bottom-up, BGR, rows padded to a multiple of four bytes. All three are required by the format
 * and all three fail silently if skipped: an upside-down image, a blue-and-red swap, and a
 * diagonal shear respectively.
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

module.exports = { decodePng, resize, encodePng, encodeBmp, canvasOf, draw }
