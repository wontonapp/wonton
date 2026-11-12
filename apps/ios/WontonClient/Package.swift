// swift-tools-version: 6.0
// Rust 客户端核心的 Swift 包装：XCFramework 和 Swift 绑定都由 scripts/build-core.sh 生成。
import PackageDescription

let package = Package(
    name: "WontonClient",
    platforms: [.iOS(.v17)],
    products: [
        .library(name: "WontonClient", targets: ["WontonClient"])
    ],
    targets: [
        .binaryTarget(name: "WontonClientFFI", path: "build/WontonClientFFI.xcframework"),
        .target(
            name: "WontonClient",
            dependencies: ["WontonClientFFI"],
            linkerSettings: [.linkedFramework("Network"), .linkedFramework("SystemConfiguration")]
        )
    ]
)
