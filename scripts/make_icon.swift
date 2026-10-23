// Renders assets/icon.png (1024x1024), the Folio app icon.
// Run: swift scripts/make_icon.swift
import AppKit

let size: CGFloat = 1024
let image = NSImage(size: NSSize(width: size, height: size))
image.lockFocus()
let ctx = NSGraphicsContext.current!.cgContext

// Rounded-square body on Apple's icon grid (824pt body, 100pt inset).
let body = CGRect(x: 100, y: 100, width: 824, height: 824)
let bodyPath = CGPath(roundedRect: body, cornerWidth: 185, cornerHeight: 185, transform: nil)
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -12), blur: 28, color: NSColor(white: 0, alpha: 0.35).cgColor)
ctx.addPath(bodyPath)
ctx.setFillColor(NSColor(srgbRed: 0.11, green: 0.12, blue: 0.14, alpha: 1).cgColor)
ctx.fillPath()
ctx.restoreGState()

ctx.saveGState()
ctx.addPath(bodyPath)
ctx.clip()
let gradient = CGGradient(colorsSpace: CGColorSpaceCreateDeviceRGB(), colors: [
    NSColor(srgbRed: 0.20, green: 0.22, blue: 0.27, alpha: 1).cgColor,
    NSColor(srgbRed: 0.10, green: 0.11, blue: 0.13, alpha: 1).cgColor,
] as CFArray, locations: [0, 1])!
ctx.drawLinearGradient(gradient, start: CGPoint(x: 512, y: 924), end: CGPoint(x: 512, y: 100), options: [])
ctx.restoreGState()

// A sheet of paper with a folded corner.
let page = CGRect(x: 292, y: 210, width: 440, height: 580)
let fold: CGFloat = 110
let sheet = CGMutablePath()
sheet.move(to: CGPoint(x: page.minX, y: page.minY))
sheet.addLine(to: CGPoint(x: page.maxX, y: page.minY))
sheet.addLine(to: CGPoint(x: page.maxX, y: page.maxY - fold))
sheet.addLine(to: CGPoint(x: page.maxX - fold, y: page.maxY))
sheet.addLine(to: CGPoint(x: page.minX, y: page.maxY))
sheet.closeSubpath()
ctx.saveGState()
ctx.setShadow(offset: CGSize(width: 0, height: -10), blur: 24, color: NSColor(white: 0, alpha: 0.45).cgColor)
ctx.addPath(sheet)
ctx.setFillColor(NSColor(srgbRed: 0.99, green: 0.98, blue: 0.96, alpha: 1).cgColor)
ctx.fillPath()
ctx.restoreGState()

let flap = CGMutablePath()
flap.move(to: CGPoint(x: page.maxX - fold, y: page.maxY))
flap.addLine(to: CGPoint(x: page.maxX - fold, y: page.maxY - fold))
flap.addLine(to: CGPoint(x: page.maxX, y: page.maxY - fold))
flap.closeSubpath()
ctx.addPath(flap)
ctx.setFillColor(NSColor(srgbRed: 0.84, green: 0.82, blue: 0.78, alpha: 1).cgColor)
ctx.fillPath()

// Serif "F" plus a few text lines.
let font = NSFont(name: "Georgia-Bold", size: 300) ?? NSFont.boldSystemFont(ofSize: 300)
let attrs: [NSAttributedString.Key: Any] = [
    .font: font,
    .foregroundColor: NSColor(srgbRed: 0.13, green: 0.13, blue: 0.15, alpha: 1),
]
let f = NSAttributedString(string: "F", attributes: attrs)
f.draw(at: CGPoint(x: page.minX + 62, y: page.minY + 230))

let accent = NSColor(srgbRed: 0.48, green: 0.64, blue: 1.0, alpha: 1)
ctx.setFillColor(accent.cgColor)
ctx.fill(CGRect(x: page.minX + 70, y: page.minY + 200, width: 300, height: 22))
ctx.setFillColor(NSColor(white: 0.78, alpha: 1).cgColor)
for (i, w) in [300.0, 250.0, 280.0].enumerated() {
    ctx.fill(CGRect(x: page.minX + 70, y: page.minY + 150 - CGFloat(i) * 42, width: w, height: 16))
}

image.unlockFocus()
let rep = NSBitmapImageRep(data: image.tiffRepresentation!)!
// Force exactly 1024px regardless of screen scale.
rep.size = NSSize(width: size, height: size)
let png = rep.representation(using: .png, properties: [:])!
try! png.write(to: URL(fileURLWithPath: "assets/icon.png"))
print("wrote assets/icon.png \(rep.pixelsWide)x\(rep.pixelsHigh)")
