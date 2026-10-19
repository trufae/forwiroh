import SwiftUI

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
        device = value["device"] as? String ?? "Device"
        connected = value["connected"] as? Bool ?? false
        path = value["path"] as? String
        policy = value["policy"] as? [String: Any] ?? [:]
    }
}
struct Tunnel: Identifiable {
    let id: String
    let peer: String
    let kind: String
    let detail: String
}
struct PairRequest: Identifiable {
    let id: String
    let name: String
    let peer: String
}
struct Invitation {
    let id: String
    let uri: String
    let expires: Date
}

@MainActor
final class Model: ObservableObject {
    @Published var peers: [Peer] = []
    @Published var tunnels: [Tunnel] = []
    @Published var pending: [PairRequest] = []
    @Published var ready = false
    @Published var starting = false
    @Published var pairing = false
    @Published var busy = false
    @Published var identity = ""
    @Published var notice = ""
    @Published var error: String?
    @Published var invitation: Invitation?
    @Published var showPairing = false
    private var core: DesktopCore?
    private var stopping = false
    private var refreshing = false
    private var refreshAgain = false

    func start() async {
        guard core == nil && !starting && !stopping else { return }
        starting = true; defer { starting = false }
        do {
            let core = try await Task.detached { try DesktopCore() }.value
            guard !stopping else { await Task.detached { core.close() }.value; return }
            self.core = core; ready = true
            core.events { [weak self] data in
                Task { @MainActor [weak self] in
                    guard let self, !self.stopping,
                          let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
                    if let detail = event["error"] as? String { self.error = detail }
                    if event["event"] as? String == "paired" {
                        self.notice = "Device paired"
                        self.invitation = nil
                    }
                    await self.refresh()
                }
            }
            await refresh()
        } catch { self.error = error.localizedDescription }
    }
    func stop() async {
        stopping = true; ready = false
        let previous = core; core = nil
        if let previous { await Task.detached { previous.close() }.value }
        while starting { try? await Task.sleep(for: .milliseconds(20)) }
    }
    func request(_ command: [String: Any]) async throws -> [String: Any] {
        guard let core, !stopping else { throw CoreError(message: "Networking is stopped") }
        let data = try JSONSerialization.data(withJSONObject: command)
        let response = try await Task.detached { try core.command(data) }.value
        return try DesktopCore.decodeResponse(response)
    }
    func refresh() async {
        guard ready else { return }
        if refreshing { refreshAgain = true; return }
        refreshing = true; defer { refreshing = false }
        repeat {
            refreshAgain = false
            do {
                let status = try await request(["op": "status"])
                identity = status["id"] as? String ?? ""
                peers = (status["peers"] as? [[String: Any]] ?? []).compactMap(Peer.init)
                tunnels = (status["tunnels"] as? [[String: Any]] ?? []).compactMap {
                    guard let id = $0["id"] as? String, let peer = $0["peer"] as? String,
                          let kind = $0["kind"] as? String, let detail = $0["detail"] as? String else { return nil }
                    return Tunnel(id: id, peer: peer, kind: kind, detail: detail)
                }
                let requests = try await request(["op": "pending"])
                pending = (requests["value"] as? [[String: Any]] ?? []).compactMap {
                    guard let id = $0["id"] as? String, let name = $0["name"] as? String,
                          let peer = $0["peer"] as? String else { return nil }
                    return PairRequest(id: id, name: name, peer: peer)
                }
            } catch { if !stopping { self.error = error.localizedDescription } }
        } while refreshAgain && ready
    }
    @discardableResult
    func run(_ command: [String: Any], notice: String) async -> Bool {
        do {
            _ = try await request(command)
            self.notice = notice
            await refresh()
            return true
        } catch { self.error = error.localizedDescription; return false }
    }
    func invite() async {
        busy = true; defer { busy = false }
        do {
            await cancelInvite()
            let data = try await request(["op": "invite"])
            guard let id = data["id"] as? String, let uri = data["uri"] as? String,
                  let expiry = data["expires"] as? Double else { throw CoreError(message: "Invalid invitation") }
            invitation = Invitation(id: id, uri: uri, expires: Date(timeIntervalSince1970: expiry))
        } catch { self.error = error.localizedDescription }
    }
    func cancelInvite() async {
        guard let invitation else { return }
        self.invitation = nil
        _ = await run(["op": "cancel_invite", "id": invitation.id], notice: "Invitation closed")
    }
    func pair(_ uri: String) async -> Bool {
        pairing = true; defer { pairing = false }
        return await run(["op": "pair", "uri": uri.trimmingCharacters(in: .whitespacesAndNewlines), "device": "macOS"], notice: "Device paired")
    }
}
