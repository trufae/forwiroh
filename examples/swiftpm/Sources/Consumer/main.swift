import Foundation
import Forwiroh

print("Forwiroh \(Forwiroh.version)")
let qr = try Forwiroh.qrCode("Forwiroh SwiftPM consumer")
precondition(qr.pixels.count == qr.width * qr.width)
#if os(macOS) || os(Linux)
let path = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
let core = try await Forwiroh.open(stateDirectory: path, name: "SwiftPM consumer")
let status = try await core.status()
print("Endpoint \(status.id)")
await core.close()
try FileManager.default.removeItem(at: path)
#endif
