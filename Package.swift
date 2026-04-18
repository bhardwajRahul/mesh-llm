// swift-tools-version: 5.9
import PackageDescription
import Foundation

let repoRoot = URL(fileURLWithPath: #filePath).deletingLastPathComponent().path
let swiftSDKRelativePath = "sdk/swift"
let ffiXCFrameworkRelativePath = "\(swiftSDKRelativePath)/Generated/MeshLLMFFI.xcframework"
let ffiXCFrameworkPath = "\(repoRoot)/\(ffiXCFrameworkRelativePath)"
let remoteFFIXCFrameworkURL = "https://github.com/Mesh-LLM/mesh-llm/releases/download/v0.63.0-rc4/MeshLLMFFI.xcframework.zip"
let remoteFFIXCFrameworkChecksum = "b9fbdc188fd1f720471f13abb943c49745fb3f1a8d4d748fbd295793f74765b1"
let forceStubFFI = ProcessInfo.processInfo.environment["MESH_SWIFT_FORCE_STUB"] == "1"
let hasLocalFFIXCFramework = FileManager.default.fileExists(atPath: ffiXCFrameworkPath)
let hasRemoteFFIXCFramework = !forceStubFFI
    && !remoteFFIXCFrameworkURL.contains("__MESH_SWIFT_RELEASE_TAG__")
    && !remoteFFIXCFrameworkChecksum.contains("__MESH_SWIFT_RELEASE_CHECKSUM__")

var meshLLMDependencies: [Target.Dependency] = []
var packageTargets: [Target] = []

if hasLocalFFIXCFramework {
    meshLLMDependencies.append("MeshLLMFFI")
    packageTargets.append(
        .binaryTarget(
            name: "MeshLLMFFI",
            path: ffiXCFrameworkRelativePath
        )
    )
} else if hasRemoteFFIXCFramework {
    meshLLMDependencies.append("MeshLLMFFI")
    packageTargets.append(
        .binaryTarget(
            name: "MeshLLMFFI",
            url: remoteFFIXCFrameworkURL,
            checksum: remoteFFIXCFrameworkChecksum
        )
    )
}

let hasFFIBinaryTarget = hasLocalFFIXCFramework || hasRemoteFFIXCFramework

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
            path: "sdk/swift/Sources/MeshLLM",
            exclude: hasFFIBinaryTarget ? [] : ["Generated"],
            linkerSettings: [
                .linkedFramework("SystemConfiguration"),
            ]
        ),
        .testTarget(
            name: "MeshLLMTests",
            dependencies: ["MeshLLM"],
            path: "sdk/swift/Tests/MeshLLMTests"
        ),
    ] + packageTargets
)
