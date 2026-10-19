import SwiftUI
import NetworkExtension
import Network

struct Peer: Identifiable {
    let id: String
    let name: String
    let device: String
    let connected: Bool
    let path: String?
    let policy: [String: Any]
    init?(_ value: [String: Any]) {
        guard let id = value["id"] as? String, let name = value["name"] as? String else { return nil }
        self.id = id; self.name = name
        device = value["device"] as? String ?? "device"
        connected = value["connected"] as? Bool ?? false
        path = value["path"] as? String
        policy = value["policy"] as? [String: Any] ?? [:]
    }
}

@MainActor
final class TCPClient: ObservableObject {
    @Published private(set) var status = "Disconnected"
    @Published private(set) var connected = false
    @Published private(set) var connecting = false
    @Published private(set) var sending = false
    @Published private(set) var output = ""
    private var connection: NWConnection?
    private var timeoutTask: Task<Void, Never>?

    func connect(host: String, port: String) {
        disconnect()
        let host = host.trimmingCharacters(in: .whitespacesAndNewlines)
        guard !host.isEmpty && !host.contains(where: { $0.isWhitespace }),
              let number = UInt16(port), number > 0,
              let port = NWEndpoint.Port(rawValue: number) else {
            status = "Enter a host and a port between 1 and 65535"
            return
        }
        output = ""
        connecting = true
        status = "Connecting to \(host):\(number)…"
        let connection = NWConnection(host: NWEndpoint.Host(host), port: port, using: .tcp)
        self.connection = connection
        connection.stateUpdateHandler = { [weak self, weak connection] state in
            Task { @MainActor [weak self, weak connection] in
                guard let self, let connection, self.connection === connection else { return }
                switch state {
                case .ready:
                    self.connecting = false
                    self.timeoutTask?.cancel()
                    self.status = "Connected to \(host):\(number)"
                    if !self.connected {
                        self.connected = true
                        self.receive(connection)
                    }
                case .waiting(let error): self.status = "Waiting: \(error.localizedDescription)"
                case .failed(let error): self.close("Failed: \(error.localizedDescription)")
                case .cancelled: self.close("Disconnected")
                default: break
                }
            }
        }
        timeoutTask = Task { [weak self, weak connection] in
            try? await Task.sleep(for: .seconds(15))
            guard !Task.isCancelled, let self, let connection,
                  self.connection === connection && !self.connected else { return }
            self.close("Connection timed out")
        }
        connection.start(queue: .global(qos: .userInitiated))
    }

    func send(_ text: String) {
        guard let connection, connected && !sending else { return }
        sending = true
        connection.send(content: Data(text.utf8), completion: .contentProcessed { [weak self, weak connection] error in
            Task { @MainActor [weak self, weak connection] in
                guard let self, let connection, self.connection === connection else { return }
                self.sending = false
                if let error { self.close("Send failed: \(error.localizedDescription)") }
                else { self.append("\n[sent] \(text)\(text.hasSuffix("\n") ? "" : "\n")") }
            }
        })
    }

    private func receive(_ connection: NWConnection) {
        connection.receive(minimumIncompleteLength: 1, maximumLength: 16_384) { [weak self, weak connection] data, _, complete, error in
            Task { @MainActor [weak self, weak connection] in
                guard let self, let connection, self.connection === connection else { return }
                if let data, !data.isEmpty { self.append(String(decoding: data, as: UTF8.self)) }
                if let error { self.close("Receive failed: \(error.localizedDescription)") }
                else if complete { self.close("Remote closed the connection") }
                else { self.receive(connection) }
            }
        }
    }

    private func append(_ text: String) {
        output += text
        if output.utf8.count > 65_536 { output = String(output.suffix(16_384)) }
    }
    func clear() { output = "" }
    func disconnect() { close("Disconnected") }
    private func close(_ status: String) {
        timeoutTask?.cancel(); timeoutTask = nil
        let previous = connection; connection = nil
        previous?.cancel()
        connected = false; connecting = false; sending = false
        self.status = status
    }
}
struct Tunnel: Identifiable {
    let id: String
    let peer: String
    let kind: String
    let detail: String
}
@MainActor
final class Model: ObservableObject {
    @Published var peers: [Peer] = []
    @Published var tunnels: [Tunnel] = []
    @Published var error: String?
    @Published var notice: String?
    @Published var ready = false
    @Published var pairing = false
    @Published var vpnStatus: NEVPNStatus = .invalid
    @Published var vpnStats = ""
    @Published var vpnError: String?
    @Published var vpnHost = ""
    private var forwardCommands: [String: [String: Any]] = [:]
    private var userStoppedVPN = false
    private var poller: Task<Void, Never>?
    private var core: RustCore?
    private var manager: NETunnelProviderManager?
    private var observer: NSObjectProtocol?
    private var opening = false
    private var handoff = false
    private var foreground = true
    private var starting = false
    var vpnStatusText: String {
        switch vpnStatus {
        case .invalid: return "Not configured"
        case .disconnected: return "Disconnected"
        case .connecting: return "Connecting…"
        case .connected: return "Connected"
        case .reasserting: return "Reconnecting…"
        case .disconnecting: return "Disconnecting…"
        @unknown default: return "Unknown status"
        }
    }
    var vpnRunning: Bool { [.connected, .connecting, .reasserting, .disconnecting].contains(vpnStatus) }

    func start() async {
        foreground = true
        guard !starting else { return }
        if observer != nil {
            vpnStatus = manager?.connection.status ?? .disconnected
            if vpnRunning { ready = vpnStatus == .connected; refreshVPNStats() } else { await openCore() }
            return
        }
        starting = true; defer { starting = false }
        do {
            let managers = try await NETunnelProviderManager.loadAllFromPreferences()
            manager = managers.first { ($0.protocolConfiguration as? NETunnelProviderProtocol)?.providerBundleIdentifier == Bundle.main.bundleIdentifier! + ".PacketTunnel" }
            vpnStatus = manager?.connection.status ?? .disconnected
            observer = NotificationCenter.default.addObserver(forName: .NEVPNStatusDidChange, object: nil, queue: .main) { [weak self] _ in
                Task { @MainActor [weak self] in
                    guard let self else { return }
                    let previous = self.vpnStatus
                    self.vpnStatus = self.manager?.connection.status ?? .disconnected
                    self.ready = self.core != nil || self.vpnStatus == .connected
                    if self.vpnStatus == .connected { self.refreshVPNStats() }
                    else if !self.vpnRunning {
                        if [.connecting, .connected, .reasserting].contains(previous), !self.userStoppedVPN {
                            self.vpnError = RustCore.vpnError
                            self.manager?.connection.fetchLastDisconnectError { [weak self] error in
                                Task { @MainActor [weak self] in
                                    guard let self, !self.vpnRunning else { return }
                                    let text = RustCore.vpnError ?? error?.localizedDescription ?? "VPN stopped before connecting. Check the app's Network Extension signing and the server version."
                                    self.vpnError = text; self.error = text
                                }
                            }
                        }
                        await self.openCore()
                    }
                }
            }
            if !vpnRunning { await openCore() } else { ready = vpnStatus == .connected; refreshVPNStats() }
            poller = Task { [weak self] in
                while !Task.isCancelled {
                    try? await Task.sleep(for: .seconds(2))
                    guard let self else { return }
                    if self.foreground && self.vpnStatus == .connected { await self.refresh() }
                }
            }
        } catch {
            self.error = error.localizedDescription
            await openCore()
        }
    }
    func suspend() async {
        foreground = false
        let previous = core; core = nil; ready = false
        if let previous { await Task.detached { previous.close() }.value }
    }
    func openCore() async {
        guard foreground && core == nil && !opening && !handoff && !vpnRunning else { return }
        opening = true; defer { opening = false }
        do {
            let core = try await Task.detached {
                // NEVPNStatus can change before the extension releases its state lock.
                // Retry only that handoff race, with a bounded wait; preserve other errors.
                for attempt in 0..<6 {
                    do { return try RustCore() }
                    catch {
                        guard attempt < 5, error.localizedDescription.contains("state is already in use") else { throw error }
                        try await Task.sleep(nanoseconds: UInt64(100_000_000) << attempt)
                    }
                }
                throw ForwirohError(message: "VPN has not released networking yet")
            }.value
            guard foreground && !vpnRunning && !handoff else {
                await Task.detached { core.close() }.value
                return
            }
            self.core = core; ready = true
            forwardCommands.removeAll()
            for command in RustCore.savedForwards {
                do {
                    let result = try await coreRequest(core, command)
                    if let id = result["id"] as? String { forwardCommands[id] = command }
                } catch { self.error = "Cannot restore forwarding: \(error.localizedDescription)" }
            }
            try RustCore.saveForwards(Array(forwardCommands.values))
            core.events { [weak self] data in
                Task { @MainActor [weak self] in
                    guard let self,
                          let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
                    if event["event"] as? String == "access_request" {
                        print("Forwiroh access: \(event)")
                        return
                    }
                    if let text = event["error"] as? String { self.error = text }
                    await self.refresh()
                }
            }
            await refresh()
        } catch { self.error = error.localizedDescription }
    }
    private func coreRequest(_ core: RustCore, _ value: [String: Any]) async throws -> [String: Any] {
        let data = try JSONSerialization.data(withJSONObject: value)
        let response = try await Task.detached { try core.command(data) }.value
        return try RustCore.decodeResponse(response)
    }
    func request(_ value: [String: Any]) async throws -> [String: Any] {
        let result: [String: Any]
        if let core {
            result = try await coreRequest(core, value)
            let op = value["op"] as? String ?? ""
            if ["forward", "reverse", "socks"].contains(op), let id = result["id"] as? String { forwardCommands[id] = value }
            if op == "stop", let id = value["id"] as? String { forwardCommands.removeValue(forKey: id) }
            if ["forward", "reverse", "socks", "stop"].contains(op) {
                try RustCore.saveForwards(Array(forwardCommands.values))
            }
        } else {
            guard vpnStatus == .connected, let session = manager?.connection as? NETunnelProviderSession else {
                throw ForwirohError(message: "Networking is not ready. Wait for VPN to connect.")
            }
            let message = try JSONSerialization.data(withJSONObject: value)
            let data: Data = try await withCheckedThrowingContinuation { continuation in
                // Both completion paths run on MainActor; take the continuation once.
                var pending: CheckedContinuation<Data, Error>? = continuation
                let timeout = Task { @MainActor in
                    try? await Task.sleep(for: .seconds(value["op"] as? String == "pair" ? 310 : 30))
                    if let next = pending { pending = nil; next.resume(throwing: ForwirohError(message: "VPN command timed out")) }
                }
                do {
                    try session.sendProviderMessage(message) { data in
                        Task { @MainActor in
                            guard let next = pending else { return }; pending = nil; timeout.cancel()
                            if let data { next.resume(returning: data) }
                            else { next.resume(throwing: ForwirohError(message: "VPN extension did not respond")) }
                        }
                    }
                } catch {
                    pending = nil; timeout.cancel(); continuation.resume(throwing: error)
                }
            }
            guard let reply = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw ForwirohError(message: "Invalid VPN response") }
            if let error = reply["error"] as? String { throw ForwirohError(message: error) }
            result = reply["data"] as? [String: Any] ?? [:]
        }
        return result
    }
    @discardableResult
    func run(_ command: [String: Any], success: String? = nil) async -> Bool {
        error = nil
        do {
            _ = try await request(command)
            await refresh()
            notice = success
            return true
        } catch {
            self.error = error.localizedDescription
            return false
        }
    }
    func refresh() async {
        guard core != nil || vpnStatus == .connected else { return }
        let statusCore = core
        let previousForwards = Set(forwardCommands.keys)
        do {
            let data = try await request(["op": "status"])
            guard core === statusCore else { return }
            updateVPNStatus(data)
            peers = (data["peers"] as? [[String: Any]] ?? []).compactMap(Peer.init)
            tunnels = (data["tunnels"] as? [[String: Any]] ?? []).compactMap { t in
                guard let id = t["id"] as? String, let peer = t["peer"] as? String,
                      let kind = t["kind"] as? String, let detail = t["detail"] as? String else { return nil }
                return Tunnel(id: id, peer: peer, kind: kind, detail: detail)
            }
            if core != nil {
                let ids = Set(tunnels.map(\.id))
                // A forward can finish while this status call is in flight. Only
                // prune profiles that existed before the snapshot was requested.
                for id in previousForwards.subtracting(ids) { forwardCommands.removeValue(forKey: id) }
                try RustCore.saveForwards(Array(forwardCommands.values))
            }
        } catch { self.error = error.localizedDescription }
    }
    func pair(_ uri: String) async {
        error = nil
        pairing = true; defer { pairing = false }
        do {
            _ = try await request(["op": "pair", "uri": uri.trimmingCharacters(in: .whitespacesAndNewlines), "device": "iOS"])
            await refresh()
        } catch { if error.localizedDescription != "pairing cancelled" { self.error = error.localizedDescription } }
    }
    func startVPN(peer: String, routes: [String], dns: String, fullInternet: Bool) async {
        guard !vpnRunning && !handoff && !opening else { return }
        handoff = true
        userStoppedVPN = false; vpnError = nil; error = nil; RustCore.saveVPNError(nil)
        defer { handoff = false }
        do {
            let manager = manager ?? NETunnelProviderManager()
            let config = NETunnelProviderProtocol()
            config.providerBundleIdentifier = Bundle.main.bundleIdentifier! + ".PacketTunnel"
            config.serverAddress = "Forwiroh"
            config.providerConfiguration = ["peer": peer, "routes": routes, "dns": dns]
            config.includeAllNetworks = fullInternet
            config.disconnectOnSleep = false
            manager.protocolConfiguration = config
            manager.localizedDescription = "Forwiroh"
            manager.isEnabled = true
            try await manager.saveToPreferences()
            try await manager.loadFromPreferences()
            self.manager = manager
            // The extension is the sole endpoint owner while VPN is active.
            let old = core; core = nil; ready = false
            if let old { await Task.detached { old.close() }.value }
            try manager.connection.startVPNTunnel()
            vpnStatus = manager.connection.status
        } catch {
            self.error = error.localizedDescription
            handoff = false
            await openCore()
        }
    }
    func stopVPN() { userStoppedVPN = true; manager?.connection.stopVPNTunnel() }
    func refreshVPNStats() { Task { await refresh() } }
    private func updateVPNStatus(_ value: [String: Any]) {
        if let vpn = value["vpn"] as? [String: Any] {
            let server = vpn["remote_ip"] as? String ?? "10.250.0.1"
            let forwards = vpn["forward_ip"] as? String ?? "10.250.0.3"
            vpnHost = "Server: \(server) · Forwarded ports: \(forwards)"
        }
        if let traffic = value["traffic"] as? [String: Any] {
            let sent = (traffic["sent"] as? NSNumber)?.int64Value ?? 0
            let received = (traffic["received"] as? NSNumber)?.int64Value ?? 0
            vpnStats = "Sent \(ByteCountFormatter.string(fromByteCount: sent, countStyle: .binary)) · Received \(ByteCountFormatter.string(fromByteCount: received, countStyle: .binary))"
        }
        for event in value["events"] as? [[String: Any]] ?? [] {
            if let error = event["error"] as? String { self.error = error }
        }
    }
}
