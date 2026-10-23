// Renders the app icon: swift assets/make-icon.swift <output.png> [size]
import AppKit

let args = CommandLine.arguments
let out = args.count > 1 ? args[1] : "icon.png"
let size = args.count > 2 ? CGFloat(Double(args[2]) ?? 1024) : 1024
let s = size / 1024 // everything below is laid out on a 1024 grid

let rep = NSBitmapImageRep(
    bitmapDataPlanes: nil, pixelsWide: Int(size), pixelsHigh: Int(size),
    bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
    colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
let ctx = NSGraphicsContext.current!.cgContext

func rect(_ x: CGFloat, _ y: CGFloat, _ w: CGFloat, _ h: CGFloat) -> NSRect {
    NSRect(x: x * s, y: y * s, width: w * s, height: h * s)
}
func color(_ hex: UInt32, _ a: CGFloat = 1) -> NSColor {
    NSColor(srgbRed: CGFloat((hex >> 16) & 0xFF) / 255, green: CGFloat((hex >> 8) & 0xFF) / 255,
            blue: CGFloat(hex & 0xFF) / 255, alpha: a)
}

// Tile: macOS icon grid (824pt square, ~185pt corners) with a drop shadow.
let tile = NSBezierPath(roundedRect: rect(100, 100, 824, 824), xRadius: 185 * s, yRadius: 185 * s)
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -10 * s), blur: 28 * s, color: color(0x000000, 0.28).cgColor)
color(0xFF7A1A).setFill()
tile.fill()
ctx.restoreGState()
NSGradient(colors: [color(0xFFBE5C), color(0xFF8C1A), color(0xF25C05)],
           atLocations: [0, 0.5, 1], colorSpace: .sRGB)!
    .draw(in: tile, angle: -90)

// Mouse body: a tall pill, white, with its own soft shadow.
let body = NSBezierPath(roundedRect: rect(337, 222, 350, 580), xRadius: 175 * s, yRadius: 175 * s)
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -14 * s), blur: 30 * s, color: color(0x7A2E00, 0.35).cgColor)
color(0xFFFFFF).setFill()
body.fill()
ctx.restoreGState()
NSGradient(colors: [color(0xFFFFFF), color(0xFFF1E4)])!.draw(in: body, angle: -90)

// Button split and scroll wheel, cut in the tile's orange and kept inside the body.
ctx.saveGState()
body.addClip()
let split = NSBezierPath()
split.move(to: NSPoint(x: 512 * s, y: 820 * s))
split.line(to: NSPoint(x: 512 * s, y: 600 * s))
split.lineWidth = 14 * s
split.lineCapStyle = .round
color(0xF47216).setStroke()
split.stroke()
let wheel = NSBezierPath(roundedRect: rect(486, 640, 52, 112), xRadius: 26 * s, yRadius: 26 * s)
color(0xF47216).setFill()
wheel.fill()
ctx.restoreGState()

NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
