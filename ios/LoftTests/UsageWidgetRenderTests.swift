import SwiftUI
import XCTest
@testable import Loft
@testable import LoftKit

/// The Usage widget drawn on the iPhone, dark and light, and its Lock Screen gauge (#489; its view model is tested in
/// LoftKitTests/UsageWidgetTests). `WIDGET_SHOTS=<folder>` also writes each drawing there as a PNG, for screenshots.
final class UsageWidgetRenderTests: XCTestCase {
    @MainActor
    func testEveryStateDrawsAtEverySizeInDarkAndLight() throws {
        var offline = UsageWidgetSnapshot.sample
        offline.phase = .unreachable
        offline.takenAt = .now.addingTimeInterval(-3 * 3600)
        let dashes = UsageWidgetSnapshot(usage: nil, system: SystemReading(cpuTempC: nil, level: nil, sampledAt: nil), weekSpend: nil, at: .now)
        let states: [(String, UsageWidgetSnapshot)] = [("ready", .sample), ("offline", offline), ("dashes", dashes)]
        var seen: [Data] = []
        func draw(_ view: some View, _ file: String) throws {
            let renderer = ImageRenderer(content: view)
            renderer.scale = 3
            let png = try XCTUnwrap(renderer.uiImage?.pngData(), file)
            seen.append(png)
            if let dir = ProcessInfo.processInfo.environment["WIDGET_SHOTS"] {
                try FileManager.default.createDirectory(atPath: dir, withIntermediateDirectories: true)
                try png.write(to: URL(fileURLWithPath: dir).appendingPathComponent(file))
            }
        }
        for (name, snapshot) in states {
            for scheme in [ColorScheme.dark, .light] {
                let tile = scheme == .dark ? Color(white: 0.11) : .white
                for (size, width) in [(UsageWidgetView.Size.small, 170.0), (.medium, 364.0)] {
                    try draw(UsageWidgetView(snapshot, size: size).padding(16).frame(width: width, height: 170).background(tile)
                        .environment(\.colorScheme, scheme), "ios-usage-\(name)-\(size)-\(scheme == .dark ? "dark" : "light").png")
                }
            }
            try draw(UsageGaugeView(snapshot).frame(width: 76, height: 76).background(Color.black).environment(\.colorScheme, .dark),
                     "ios-usage-\(name)-lock.png")
        }
        XCTAssertEqual(Set(seen).count, seen.count, "no two states or sizes draw the same")
    }
}
