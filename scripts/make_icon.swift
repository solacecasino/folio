// Renders assets/icon.png (1024x1024) and assets/icon-256.png: the
// text9000 mark, three lines of text on a graphite body, the last line cut
// on splice9000's slant into ink green, as the 9000 series' icons are.
// Run: swift scripts/make_icon.swift   (then python3 scripts/make_ico.py)
import AppKit

func render(_ size: Int) -> NSBitmapImageRep {
    let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: size, pixelsHigh: size,
                               bitsPerSample: 8, samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                               colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
    NSGraphicsContext.saveGraphicsState()
    NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
    let ctx = NSGraphicsContext.current!.cgContext
    let k = CGFloat(size) / 1024
    ctx.scaleBy(x: k, y: k)
    func color(_ hex: UInt32, _ a: CGFloat = 1) -> CGColor {
        NSColor(srgbRed: CGFloat(hex >> 16 & 0xff) / 255, green: CGFloat(hex >> 8 & 0xff) / 255,
                blue: CGFloat(hex & 0xff) / 255, alpha: a).cgColor
    }

    // Body on Apple's icon grid: 824 square, 100 inset.
    let body = CGPath(roundedRect: CGRect(x: 100, y: 100, width: 824, height: 824), cornerWidth: 185, cornerHeight: 185, transform: nil)
    ctx.saveGState()
    ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: color(0, 0.35))
    ctx.addPath(body)
    ctx.setFillColor(color(0x17181b))
    ctx.fillPath()
    ctx.restoreGState()
    ctx.saveGState()
    ctx.addPath(body)
    ctx.clip()
    let grad = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: [color(0x22242a), color(0x121316)] as CFArray, locations: [0, 1])!
    ctx.drawLinearGradient(grad, start: CGPoint(x: 0, y: 924), end: CGPoint(x: 0, y: 100), options: [])
    ctx.restoreGState()

    // Three lines of text, top to bottom (y grows upward here).
    let (left, right): (CGFloat, CGFloat) = (226, 798)
    let h: CGFloat = 92, r: CGFloat = 34
    let rows: [CGFloat] = [668, 466, 264]
    func bar(_ x0: CGFloat, _ x1: CGFloat, _ y: CGFloat, _ fill: CGColor) {
        ctx.addPath(CGPath(roundedRect: CGRect(x: x0, y: y, width: x1 - x0, height: h), cornerWidth: r, cornerHeight: r, transform: nil))
        ctx.setFillColor(fill)
        ctx.fillPath()
    }
    bar(left, right, rows[0], color(0xe8e9eb))
    bar(left, right - 120, rows[1], color(0xe8e9eb))

    // The last line: light, a slanted cut, then green.
    let y = rows[2], slant: CGFloat = 30, gap: CGFloat = 22, cut: CGFloat = 470, end: CGFloat = right - 40
    func piece(_ points: [CGPoint], _ fill: CGColor) {
        let p = CGMutablePath()
        p.addLines(between: points)
        p.closeSubpath()
        ctx.saveGState()
        ctx.addPath(CGPath(roundedRect: CGRect(x: left, y: y, width: end - left, height: h), cornerWidth: r, cornerHeight: r, transform: nil))
        ctx.clip()
        ctx.addPath(p)
        ctx.setFillColor(fill)
        ctx.fillPath()
        ctx.restoreGState()
    }
    piece([CGPoint(x: left, y: y + h), CGPoint(x: cut - gap / 2 - slant, y: y + h), CGPoint(x: cut - gap / 2 + slant, y: y), CGPoint(x: left, y: y)], color(0xe8e9eb))
    piece([CGPoint(x: cut + gap / 2 - slant, y: y + h), CGPoint(x: end, y: y + h), CGPoint(x: end, y: y), CGPoint(x: cut + gap / 2 + slant, y: y)], color(0x3fd68f))
    NSGraphicsContext.restoreGraphicsState()
    return rep
}

let dir = URL(fileURLWithPath: CommandLine.arguments.count > 1 ? CommandLine.arguments[1] : "assets")
for (size, name) in [(1024, "icon.png"), (256, "icon-256.png"), (48, "icon-48.png"), (32, "icon-32.png"), (16, "icon-16.png")] {
    try! render(size).representation(using: .png, properties: [:])!.write(to: dir.appendingPathComponent(name))
}
print("wrote the icons in \(dir.path)")
