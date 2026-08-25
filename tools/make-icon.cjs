/**
 * Generates the source app icon as a PNG, with no image-library dependency.
 *
 * A rounded square in the Apple system blue with a white envelope-flap chevron. Original
 * geometry — docs/05 §1 rules out copying Apple's stamp artwork or app icon.
 */
const fs = require('node:fs')
const zlib = require('node:zlib')

const SIZE = 1024
const SS = 4 // supersample factor for antialiasing

const BLUE = [0x00, 0x7a, 0xff]
const WHITE = [0xff, 0xff, 0xff]

const CRC_TABLE = (() => {
  const t = new Int32Array(256)
  for (let n = 0; n < 256; n++) {
    let c = n
    for (let k = 0; k < 8; k++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1
    t[n] = c
  }
  return t
})()

function crc32(buf) {
  let c = -1
  for (const byte of buf) c = CRC_TABLE[(c ^ byte) & 0xff] ^ (c >>> 8)
  return (c ^ -1) >>> 0
}

function chunk(type, data) {
  const len = Buffer.alloc(4)
  len.writeUInt32BE(data.length, 0)
  const body = Buffer.concat([Buffer.from(type, 'ascii'), data])
  const crc = Buffer.alloc(4)
  crc.writeUInt32BE(crc32(body), 0)
  return Buffer.concat([len, body, crc])
}

function encodePng(width, height, rgba) {
  const stride = width * 4
  const raw = Buffer.alloc((stride + 1) * height)
  for (let y = 0; y < height; y++) {
    raw[y * (stride + 1)] = 0 // filter: none
    rgba.copy(raw, y * (stride + 1) + 1, y * stride, (y + 1) * stride)
  }
  const ihdr = Buffer.alloc(13)
  ihdr.writeUInt32BE(width, 0)
  ihdr.writeUInt32BE(height, 4)
  ihdr[8] = 8 // bit depth
  ihdr[9] = 6 // colour type: RGBA
  return Buffer.concat([
    Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]),
    chunk('IHDR', ihdr),
    chunk('IDAT', zlib.deflateSync(raw, { level: 9 })),
    chunk('IEND', Buffer.alloc(0)),
  ])
}

/** Signed distance from a point to a line segment. */
function distanceToSegment(px, py, ax, ay, bx, by) {
  const abx = bx - ax
  const aby = by - ay
  const apx = px - ax
  const apy = py - ay
  const lenSq = abx * abx + aby * aby
  let t = lenSq === 0 ? 0 : (apx * abx + apy * aby) / lenSq
  t = Math.max(0, Math.min(1, t))
  const dx = px - (ax + t * abx)
  const dy = py - (ay + t * aby)
  return Math.hypot(dx, dy)
}

function insideRoundedRect(x, y, size, radius) {
  const inset = 0
  const min = inset
  const max = size - inset
  const cx = Math.min(Math.max(x, min + radius), max - radius)
  const cy = Math.min(Math.max(y, min + radius), max - radius)
  return Math.hypot(x - cx, y - cy) <= radius
}

const hi = SIZE * SS
const radius = hi * 0.225

// Envelope: a stroked rounded rectangle for the body, and a V for the flap running
// from its two top corners down to the centre. Original geometry — docs/05 §1 rules
// out copying Apple's stamp artwork.
const bodyHalfW = hi * 0.28
const bodyHalfH = hi * 0.18
const bodyRadius = hi * 0.045
const strokeHalf = hi * 0.034

const topY = hi * 0.5 - bodyHalfH
const leftX = hi * 0.5 - bodyHalfW
const rightX = hi * 0.5 + bodyHalfW
const flapApexY = topY + bodyHalfH * 1.25

/** Signed distance to a rounded rectangle centred on the canvas. */
function sdRoundedRect(px, py) {
  const dx = Math.abs(px - hi * 0.5) - (bodyHalfW - bodyRadius)
  const dy = Math.abs(py - hi * 0.5) - (bodyHalfH - bodyRadius)
  const ox = Math.max(dx, 0)
  const oy = Math.max(dy, 0)
  return Math.hypot(ox, oy) + Math.min(Math.max(dx, dy), 0) - bodyRadius
}

const rgba = Buffer.alloc(SIZE * SIZE * 4)

for (let y = 0; y < SIZE; y++) {
  for (let x = 0; x < SIZE; x++) {
    let rSum = 0
    let gSum = 0
    let bSum = 0
    let aSum = 0

    for (let sy = 0; sy < SS; sy++) {
      for (let sx = 0; sx < SS; sx++) {
        const px = x * SS + sx + 0.5
        const py = y * SS + sy + 0.5

        if (!insideRoundedRect(px, py, hi, radius)) continue

        const onBody = Math.abs(sdRoundedRect(px, py)) <= strokeHalf

        const onFlap =
          Math.min(
            distanceToSegment(px, py, leftX, topY, hi * 0.5, flapApexY),
            distanceToSegment(px, py, hi * 0.5, flapApexY, rightX, topY),
          ) <= strokeHalf
        const colour = onBody || onFlap ? WHITE : BLUE
        rSum += colour[0]
        gSum += colour[1]
        bSum += colour[2]
        aSum += 255
      }
    }

    const samples = SS * SS
    const i = (y * SIZE + x) * 4
    if (aSum === 0) {
      rgba[i] = rgba[i + 1] = rgba[i + 2] = rgba[i + 3] = 0
    } else {
      // Premultiplied average over covered samples, alpha from coverage.
      const covered = aSum / 255
      rgba[i] = Math.round(rSum / covered)
      rgba[i + 1] = Math.round(gSum / covered)
      rgba[i + 2] = Math.round(bSum / covered)
      rgba[i + 3] = Math.round(aSum / samples)
    }
  }
}

const out = process.argv[2]
if (!out) throw new Error('usage: node make-icon.cjs <output.png>')
fs.writeFileSync(out, encodePng(SIZE, SIZE, rgba))
console.log(`wrote ${out} (${SIZE}x${SIZE})`)
