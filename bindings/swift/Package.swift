// swift-tools-version: 5.9
import PackageDescription
import Foundation

let ffiXCFrameworkPath = "Generated/mesh_ffiFFI.xcframework"
let hasFFIXCFramework = FileManager.default.fileExists(atPath: ffiXCFrameworkPath)

var meshLLMDependencies: [Target.Dependency] = []
var packageTargets: [Target] = []

if hasFFIXCFramework {
    meshLLMDependencies.append("mesh_ffiFFI")
    packageTargets.append(
        .binaryTarget(
            name: "mesh_ffiFFI",
            path: ffiXCFrameworkPath
        )
    )
}

let package = Package(
    name: "MeshLLM",
    platforms: [
        .iOS(.v16),
        .macOS(.v13),
    ],
    products: [
        .library(
            name: "MeshLLM",
            targets: ["MeshLLM"]
        ),
    ],
    targets: [
        .target(
            name: "MeshLLM",
            dependencies: meshLLMDependencies,
            path: "Sources/MeshLLM",
            exclude: hasFFIXCFramework ? [] : ["Generated"]
        ),
        .testTarget(
            name: "MeshLLMTests",
            dependencies: ["MeshLLM"],
            path: "Tests/MeshLLMTests"
        ),
    ] + packageTargets
)
