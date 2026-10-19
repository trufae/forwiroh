import Foundation

struct CoreError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// Registers every caller before accessing the pointer; shutdown joins them all.
final class DesktopCore: @unchecked Sendable {
    static let version = String(cString: forwiroh_version()!)
    static var statePath: String {
        ProcessInfo.processInfo.environment["FORWIROH_STATE"] ??
        FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask)[0]
            .appendingPathComponent("Forwiroh").path
    }
    static var cryptoInfo: [String: Any] {
        guard let raw = forwiroh_crypto_info() else { return [:] }
        defer { forwiroh_string_free(raw) }
        return (try? JSONSerialization.jsonObject(with: Data(String(cString: raw).utf8))) as? [String: Any] ?? [:]
    }
    private let handle: OpaquePointer
    private let lock = NSLock()
    private let calls = DispatchGroup()
    private var closing = false

    init(path: String = DesktopCore.statePath, name: String = "Mac") throws {
        var error: UnsafeMutablePointer<CChar>?
        let pointer = path.withCString { path in
            name.withCString { forwiroh_open_desktop(path, $0, &error) }
        }
        guard let pointer else {
            defer { forwiroh_string_free(error) }
            let raw = error.map { String(cString: $0) } ?? "Cannot start networking"
            let value = try? JSONSerialization.jsonObject(with: Data(raw.utf8)) as? [String: Any]
            throw CoreError(message: value?["error"] as? String ?? raw)
        }
        handle = pointer
    }
    private func enter() throws {
        lock.lock(); defer { lock.unlock() }
        guard !closing else { throw CoreError(message: "Networking is stopped") }
        calls.enter()
    }
    func command(_ bytes: Data) throws -> Data {
        try enter(); defer { calls.leave() }
        let raw = String(decoding: bytes, as: UTF8.self).withCString { forwiroh_command(handle, $0) }
        defer { forwiroh_string_free(raw) }
        guard let raw else { throw CoreError(message: "Invalid core response") }
        return Data(String(cString: raw).utf8)
    }
    static func decodeResponse(_ data: Data) throws -> [String: Any] {
        guard let result = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw CoreError(message: "Invalid core response")
        }
        if let error = result["error"] as? String { throw CoreError(message: error) }
        return result["data"] as? [String: Any] ?? ["value": result["data"] ?? NSNull()]
    }
    func events(_ receive: @escaping @Sendable (Data) -> Void) {
        do { try enter() } catch { return }
        DispatchQueue.global(qos: .utility).async { [self] in
            defer { calls.leave() }
            while let raw = forwiroh_next_event(handle) {
                let data = Data(String(cString: raw).utf8)
                forwiroh_string_free(raw)
                receive(data)
            }
        }
    }
    /// Off the UI thread; no caller can enter once closing begins.
    func close() {
        lock.lock()
        if closing { lock.unlock(); return }
        closing = true; lock.unlock()
        forwiroh_shutdown(handle)
        calls.wait()
        forwiroh_free(handle)
    }
}
