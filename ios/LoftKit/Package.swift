// swift-tools-version: 5.10
import PackageDescription

// The code the iPhone and Mac apps share: models, the API client, the store, the on-device model and
// the voice pipeline. Anything only one platform has sits in Sources/LoftKit/Platform.
let package = Package(
    name: "LoftKit",
    platforms: [.iOS(.v17), .macOS(.v14)],
    products: [
        .library(name: "LoftKit", targets: ["LoftKit"]),
        // Transcribes a recording on the Mac for the backend (`POST /api/voice/transcribe`, spec #493).
        .executable(name: "loft-transcribe", targets: ["loft-transcribe"]),
    ],
    targets: [
        .target(name: "LoftKit"),
        .executableTarget(name: "loft-transcribe", dependencies: ["LoftKit"]),
        // `swift test` runs these on the Mac: what only the Mac draws or does (the widgets, spec #389).
        .testTarget(name: "LoftKitTests", dependencies: ["LoftKit"]),
    ]
)
