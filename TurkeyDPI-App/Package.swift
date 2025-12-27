// swift-tools-version: 5.9
import PackageDescription

let package = Package(
    name: "TurkeyDPI",
    platforms: [
        .macOS(.v14)
    ],
    targets: [
        .executableTarget(
            name: "TurkeyDPI",
            path: "Sources"
        )
    ]
)
