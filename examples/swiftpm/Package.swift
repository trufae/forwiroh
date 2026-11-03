// swift-tools-version: 6.2
import PackageDescription

let package = Package(
    name: "ForwirohConsumer",
    platforms: [.macOS(.v14), .iOS(.v17)],
    dependencies: [.package(name: "Forwiroh", path: Context.environment["FORWIROH_PACKAGE_PATH"] ?? "../..")],
    targets: [.executableTarget(name: "Consumer", dependencies: [
        .product(name: "Forwiroh", package: "Forwiroh"),
    ])]
)
