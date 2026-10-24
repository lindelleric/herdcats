// swift-tools-version: 5.9
// xtool build manifest (Linux). The Xcode project (project.yml) remains the
// upstream build path; this manifest only wraps the same sources for xtool.

import PackageDescription

let package = Package(
    name: "Herdcats",
    platforms: [
        .iOS(.v17),
        .macOS(.v14),
    ],
    products: [
        // xtool expects exactly one library product representing the app.
        .library(name: "Herdcats", targets: ["Herdcats"]),
    ],
    dependencies: [
        .package(path: "ThirdParty/Citadel"),
    ],
    targets: [
        .target(
            name: "Herdcats",
            dependencies: [
                .product(name: "Citadel", package: "Citadel"),
            ],
            path: "Herdcats",
            exclude: [
                "Assets.xcassets",
                "Info.plist",
                "Resources",
            ],
            swiftSettings: [
                // Xcode enables bare /regex/ literals by default; SwiftPM does not in Swift 5 mode.
                .enableUpcomingFeature("BareSlashRegexLiterals"),
            ]
        ),
    ]
)
