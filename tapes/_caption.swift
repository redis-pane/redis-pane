// Render one caption band as a PNG: `swiftc`-free, run as `swift tapes/_caption.swift "text" out.png`.
// The band is 1200x64, the brand's dark (#16181C) with a teal (#1AA088) rule on top and the text
// in the brand's light (#E9EBEE), matching site/app/social.png/route.tsx. Used by render.sh for
// `social` only, because this ffmpeg build has no drawtext (no freetype).
import AppKit

let args = CommandLine.arguments
guard args.count == 3 else { FileHandle.standardError.write("usage: _caption.swift TEXT OUT.png\n".data(using: .utf8)!); exit(2) }
let (text, out) = (args[1], args[2])
let w = 1200, h = 64
let rep = NSBitmapImageRep(bitmapDataPlanes: nil, pixelsWide: w, pixelsHigh: h, bitsPerSample: 8,
                           samplesPerPixel: 4, hasAlpha: true, isPlanar: false,
                           colorSpaceName: .deviceRGB, bytesPerRow: 0, bitsPerPixel: 0)!
NSGraphicsContext.saveGraphicsState()
NSGraphicsContext.current = NSGraphicsContext(bitmapImageRep: rep)
NSColor(srgbRed: 0x16/255.0, green: 0x18/255.0, blue: 0x1C/255.0, alpha: 1).setFill()
NSRect(x: 0, y: 0, width: w, height: h).fill()
NSColor(srgbRed: 0x1A/255.0, green: 0xA0/255.0, blue: 0x88/255.0, alpha: 1).setFill()
NSRect(x: 0, y: h - 4, width: w, height: 4).fill()
let font = NSFont(name: "HelveticaNeue-Bold", size: 32) ?? NSFont.boldSystemFont(ofSize: 32)
let attrs: [NSAttributedString.Key: Any] = [
  .font: font, .foregroundColor: NSColor(srgbRed: 0xE9/255.0, green: 0xEB/255.0, blue: 0xEE/255.0, alpha: 1)]
let s = NSAttributedString(string: text, attributes: attrs)
let size = s.size()
s.draw(at: NSPoint(x: (CGFloat(w) - size.width) / 2, y: (CGFloat(h) - 4 - size.height) / 2))
NSGraphicsContext.restoreGraphicsState()
try! rep.representation(using: .png, properties: [:])!.write(to: URL(fileURLWithPath: out))
