// swift-tools-version:5.7
import PackageDescription

let package = Package(
  name: "tauri-plugin-keylink",
  platforms: [
    .macOS(.v10_13),
    .iOS(.v16),
  ],
  products: [
    .library(name: "tauri-plugin-keylink", type: .static, targets: ["tauri-plugin-keylink"])
  ],
  dependencies: [
    .package(name: "Tauri", path: "../.tauri/tauri-api")
  ],
  targets: [
    .target(
      name: "tauri-plugin-keylink",
      dependencies: [.byName(name: "Tauri")],
      path: "Sources")
  ]
)
