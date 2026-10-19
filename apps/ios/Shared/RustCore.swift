import Foundation
import Security
import UIKit

struct ForwirohError: LocalizedError {
    let message: String
    var errorDescription: String? { message }
}

/// Owns the C handle and joins all in-flight calls before freeing it.
final class RustCore: @unchecked Sendable {
    static let version = String(cString: forwiroh_version())
    static var cryptoInfo: [String: Any] {
        let raw = forwiroh_crypto_info()
        defer { forwiroh_string_free(raw) }
        let data = Data(String(cString: raw).utf8)
        return (try? JSONSerialization.jsonObject(with: data) as? [String: Any]) ?? [:]
    }
    private let handle: OpaquePointer
    private let lock = NSLock()
    private let calls = DispatchGroup()
    private var closing = false

    static var stateURL: URL {
        get throws {
            guard let group = Bundle.main.object(forInfoDictionaryKey: "ForwirohAppGroup") as? String,
                  let root = FileManager.default.containerURL(forSecurityApplicationGroupIdentifier: group) else {
                throw ForwirohError(message: "App Group unavailable. Install an app signed with the Forwiroh entitlements.")
            }
            return root.appendingPathComponent("State", isDirectory: true)
        }
    }
    static func saveVPNError(_ message: String?) {
        guard let root = try? stateURL else { return }
        try? FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        let value: [String: Any] = ["error": message ?? ""]
        if let data = try? JSONSerialization.data(withJSONObject: value) {
            try? data.write(to: root.appendingPathComponent("vpn-error.json"), options: .atomic)
        }
    }
    static var vpnError: String? {
        guard let root = try? stateURL, let data = try? Data(contentsOf: root.appendingPathComponent("vpn-error.json")),
              let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
              let error = value["error"] as? String, !error.isEmpty else { return nil }
        return error
    }
    static func saveForwards(_ commands: [[String: Any]]) throws {
        let root = try stateURL
        try FileManager.default.createDirectory(at: root, withIntermediateDirectories: true)
        try JSONSerialization.data(withJSONObject: commands).write(to: root.appendingPathComponent("vpn-forwards.json"), options: .atomic)
    }
    static var savedForwards: [[String: Any]] {
        guard let root = try? stateURL, let data = try? Data(contentsOf: root.appendingPathComponent("vpn-forwards.json")),
              let commands = try? JSONSerialization.jsonObject(with: data) as? [[String: Any]] else { return [] }
        return commands
    }
    private static func identity() throws -> Data {
        guard let group = Bundle.main.object(forInfoDictionaryKey: "ForwirohKeychainGroup") as? String else {
            throw ForwirohError(message: "Missing Keychain access group")
        }
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: "Forwiroh identity", kSecAttrAccount as String: "iroh/1",
            kSecAttrAccessGroup as String: group]
        var lookup = query
        lookup[kSecReturnData as String] = true
        var result: CFTypeRef?
        let status = SecItemCopyMatching(lookup as CFDictionary, &result)
        if status == errSecSuccess, let data = result as? Data, data.count == 32 { return data }
        guard status == errSecItemNotFound else { throw ForwirohError(message: "Keychain unavailable (\(status))") }
        var data = Data(count: 32)
        let randomStatus = data.withUnsafeMutableBytes { SecRandomCopyBytes(kSecRandomDefault, 32, $0.baseAddress!) }
        guard randomStatus == errSecSuccess else { throw ForwirohError(message: "Cannot create identity") }
        var insert = query
        insert[kSecValueData as String] = data
        insert[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlockThisDeviceOnly
        let added = SecItemAdd(insert as CFDictionary, nil)
        if added == errSecDuplicateItem { return try identity() }
        guard added == errSecSuccess else { throw ForwirohError(message: "Cannot save identity (\(added))") }
        return data
    }
    init() throws {
        let path = try Self.stateURL.path
        let key = try Self.identity()
        var error: UnsafeMutablePointer<CChar>?
        let pointer = key.withUnsafeBytes { bytes in
            path.withCString { path in
                // UIDevice.name can contain more than the protocol's 64-byte name limit.
                "iPhone".withCString { name in forwiroh_open(path, name, bytes.bindMemory(to: UInt8.self).baseAddress, &error) }
            }
        }
        guard let pointer else {
            let raw = error.map { String(cString: $0) } ?? "Cannot start networking"
            let detail = raw.data(using: .utf8).flatMap { try? JSONSerialization.jsonObject(with: $0) as? [String: Any] }
            let message = detail?["error"] as? String ?? raw
            forwiroh_string_free(error)
            throw ForwirohError(message: message)
        }
        handle = pointer
    }
    private func enter() throws {
        lock.lock(); defer { lock.unlock() }
        guard !closing else { throw ForwirohError(message: "Networking is stopped") }
        calls.enter()
    }
    func command(_ value: [String: Any]) throws -> [String: Any] {
        try Self.decodeResponse(command(JSONSerialization.data(withJSONObject: value)))
    }
    func command(_ data: Data) throws -> Data {
        try enter(); defer { calls.leave() }
        guard let json = String(data: data, encoding: .utf8) else { throw ForwirohError(message: "Invalid command") }
        let raw = json.withCString { forwiroh_command(handle, $0) }
        defer { forwiroh_string_free(raw) }
        guard let raw else { throw ForwirohError(message: "Invalid core response") }
        return Data(String(cString: raw).utf8)
    }
    static func decodeResponse(_ data: Data) throws -> [String: Any] {
        guard let result = try JSONSerialization.jsonObject(with: data) as? [String: Any] else {
            throw ForwirohError(message: "Invalid core response")
        }
        if let error = result["error"] as? String { throw ForwirohError(message: error) }
        if let body = result["data"] as? [String: Any] { return body }
        return ["value": result["data"] ?? NSNull()]
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
    func sendPacket(session: String, data: Data) -> Bool {
        do { try enter() } catch { return false }
        defer { calls.leave() }
        return session.withCString { session in
            data.withUnsafeBytes { b in forwiroh_send_packet(handle, session, b.bindMemory(to: UInt8.self).baseAddress, data.count) == 0 }
        }
    }
    func receivePacket(session: String) -> Data? {
        do { try enter() } catch { return nil }
        defer { calls.leave() }
        var bytes = [UInt8](repeating: 0, count: 65_535)
        let count = session.withCString { id in
            bytes.withUnsafeMutableBufferPointer { buffer in
                forwiroh_receive_packet(handle, id, buffer.baseAddress, buffer.count)
            }
        }
        return count > 0 ? Data(bytes.prefix(Int(count))) : nil
    }
    /// Must run off the main thread. No new calls can enter after this point.
    func close() {
        lock.lock()
        if closing { lock.unlock(); return }
        closing = true; lock.unlock()
        forwiroh_shutdown(handle)
        calls.wait()
        forwiroh_free(handle)
    }
}
