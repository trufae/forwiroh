import Foundation
import Testing
@testable import Forwiroh

@Test func capabilitiesAndQR() throws {
    #expect(!Forwiroh.version.isEmpty)
    #expect(try Forwiroh.cryptoInfo()["identity_algorithm"]?.string == "Ed25519")
    let qr = try Forwiroh.qrCode("forwiroh SwiftPM")
    #expect(qr.width > 0)
    #expect(qr.pixels.count == qr.width * qr.width)
    #expect(qr.pixels.first == true)
    #expect(throws: ForwirohError.self) { try Forwiroh.qrCode("before\0after") }
    #expect(throws: ForwirohError.self) { try Forwiroh.qrCode(String(repeating: "x", count: 5000)) }
}

@Test func sharedSuccessResponsesPreserveUnknownPolicyFields() throws {
    let fixture = URL(fileURLWithPath: #filePath).deletingLastPathComponent().deletingLastPathComponent().appendingPathComponent("Fixtures/control-success.json")
    let values = try JSONDecoder().decode([String: JSONValue].self, from: Data(contentsOf: fixture))
    for key in ["status_legacy", "status_current"] { #expect(try values[key]!.decode(Status.self).id == "node") }
    #expect(try values["invitation"]!.decode(Invitation.self).id == "invitation")
    #expect(try values["pending"]!.decode([PairingRequest].self).first?.id == "request")
    for key in ["vpn_legacy", "vpn_current"] { #expect(try values[key]!.decode(VPNSession.self).sessionID.rawValue == "session") }
    var policy = try values["policy_future"]!.decode(PeerPolicy.self)
    policy.restricted = false
    let encoded = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(policy))
    #expect(encoded["future_permission"] == values["policy_future"]?["future_permission"])
    #expect(encoded["vpn"]?["future_permission"] == .bool(false))
    let legacy = try values["policy_legacy"]!.decode(PeerPolicy.self)
    #expect(!legacy.restricted && legacy.socksPorts.isEmpty)
    let payload = values["event"]!
    let event = CoreEvent(name: payload["event"]!.string!, payload: payload)
    #expect(event.name == "future_event" && event.payload["opaque"]?["values"] == .array([.integer(1), .bool(true)]))
}

@Test func responseShapesAndErrors() throws {
    #expect(try response(.object(["data": .null])) == .null)
    #expect(try response(.object(["data": .array([])])) == .array([]))
    #expect(throws: ForwirohError("denied")) { try response(.object(["error": .string("denied")])) }
    #expect(ForwirohError("denied").code == nil)
    #expect(throws: ForwirohError("denied")) {
        try response(.object(["error": .string("denied"), "error_code": .string("operation_failed")]))
    }
    for code in ["invalid_command", "wrong_tunnel_kind", "unsupported_backend", "core_stopped", "operation_failed", "future_code"] {
        do {
            _ = try response(.object(["error": .string("denied"), "error_code": .string(code)]))
            Issue.record("A core error must throw")
        } catch let error as ForwirohError {
            #expect(error.code == code)
            #expect(error.message == "denied")
        }
    }
    #expect(throws: ForwirohError.self) { try response(.object([:])) }
    let policy = PeerPolicy(restricted: true, services: ["ssh": "127.0.0.1:22"], socksPorts: [443])
    let data = try JSONEncoder().encode(policy)
    let value = try JSONDecoder().decode(JSONValue.self, from: data)
    #expect(value["socks_ports"] == .array([.integer(443)]))
    #expect(try value.decode(PeerPolicy.self) == policy)
}

@Test func invalidIdentityAndCString() async throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    await #expect(throws: ForwirohError.self) {
        try await Forwiroh.open(stateDirectory: path, name: "test", identity: Data(count: 31))
    }
    await #expect(throws: ForwirohError.self) {
        try await Forwiroh.open(stateDirectory: path, name: "before\0after")
    }
    #expect(!FileManager.default.fileExists(atPath: path.path))
}

@Test func vpnIdentifiersDecodeBothLegacyAndExplicitFields() throws {
    let legacy: JSONValue = .object([
        "id": .string("session"), "local_ip": .string("10.250.0.2"), "remote_ip": .string("10.250.0.1"),
        "local_ipv6": .string("fd00:fa::2"), "remote_ipv6": .string("fd00:fa::1"),
        "forward_ip": .string("10.250.0.3"), "routes": .array([]), "mtu": .integer(1280)
    ])
    let old = try legacy.decode(VPNSession.self)
    #expect(old.id == "session" && old.sessionID.rawValue == "session" && old.tunnelID == nil)
    guard case .object(var fields) = legacy else { return }
    fields["session_id"] = .string("session"); fields["tunnel_id"] = .string("tunnel")
    let current = try JSONValue.object(fields).decode(VPNSession.self)
    #expect(current.sessionID == VPNSessionID(rawValue: "session"))
    #expect(current.tunnelID == TunnelID(rawValue: "tunnel"))
    fields["session_id"] = .string("conflict")
    #expect(throws: DecodingError.self) { try JSONValue.object(fields).decode(VPNSession.self) }
}

#if os(macOS) || os(Linux) || os(Windows)
@Test(.timeLimit(.minutes(1))) func lifecyclePersistenceAndConcurrentShutdown() async throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: path) }
    let core = try await Forwiroh.open(stateDirectory: path, name: "SwiftPM")
    let snapshot = try await core.status()
    let api = try await core.apiInfo()
    #expect(api.controlAPIVersion == 1)
    #expect(api.cABIVersion == 1)
    #expect(api.maxPacketSize == 65_535)
    #expect(api.supports("forward_start") && api.supports("forward") && api.supports("stop"))
    #expect(api.backends.contains("vpn_packet"))
    #expect(snapshot.name == "SwiftPM")
    #expect(snapshot.peers.isEmpty)
    let invitation = try await core.invite()
    #expect(invitation.uri.hasPrefix("forwiroh:"))
    try await core.cancelInvitation(id: invitation.id)
    #expect(try await core.pendingPairings().isEmpty)
    await #expect(throws: ForwirohError.self) {
        try await Forwiroh.open(stateDirectory: path, name: "duplicate")
    }
    do {
        _ = try await core.command(["op": .string("not_a_command")])
        Issue.record("An unknown command must fail")
    } catch let error as ForwirohError {
        #expect(error.code == "invalid_command")
    }
    do {
        _ = try await core.receivePacket(session: "missing")
        Issue.record("An unknown packet session must fail")
    } catch let error as ForwirohError {
        #expect(error.code == "session_not_found")
    }
    let reader = Task { for await _ in core.events {} }
    await withTaskGroup(of: Void.self) { group in
        for _ in 0..<8 { group.addTask { _ = try? await core.status() } }
        for _ in 0..<4 { group.addTask { await core.close() } }
    }
    await reader.value
    do {
        _ = try await core.status()
        Issue.record("A stopped core must fail")
    } catch let error as ForwirohError {
        #expect(error.code == "core_stopped")
    }
    let reopened = try await Forwiroh.open(stateDirectory: path, name: "second")
    #expect(try await reopened.status().id == snapshot.id)
    await reopened.close()
}

@Test(.timeLimit(.minutes(1))) func droppingOwnerReleasesDirectoryWithEventReader() async throws {
    let path = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: path) }
    var core: Forwiroh? = try await Forwiroh.open(stateDirectory: path, name: "owner")
    let events = core!.events
    core = nil
    for await _ in events {}
    // The open worker can briefly retain its result after resuming our task.
    // If it performs deinit, the event reader can finish before shutdown frees
    // the handle and its directory lock. Verify eventual cleanup, not ordering
    // between those threads; explicit close() is the synchronous reuse barrier.
    let deadline = ContinuousClock.now.advanced(by: .seconds(5))
    while true {
        do {
            let reopened = try await Forwiroh.open(stateDirectory: path, name: "new-owner")
            await reopened.close()
            return
        } catch let error as ForwirohError
            where error.message == "state is already in use; use the running daemon"
                && ContinuousClock.now < deadline {
            try await Task.sleep(for: .milliseconds(10))
        }
    }
}

@Test(.timeLimit(.minutes(1))) func pairingAndTunnelCommands() async throws {
    let root = FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)
    defer { try? FileManager.default.removeItem(at: root) }
    let server = try await Forwiroh.open(stateDirectory: root.appendingPathComponent("server"), name: "server")
    let client = try await Forwiroh.open(stateDirectory: root.appendingPathComponent("client"), name: "client")
    do {
        try await server.setPairingPolicy(PeerPolicy(restricted: true, services: ["pmai-acp": "127.0.0.1:19283"]))
        try await client.setPairingPolicy(PeerPolicy(restricted: true))
        let invitation = try await server.invite()
        let pairing = Task { try await client.pair(invitation: invitation.uri, device: "SwiftPM") }
        var pending: [PairingRequest] = []
        for _ in 0..<100 {
            pending = try await server.pendingPairings()
            if !pending.isEmpty { break }
            try await Task.sleep(for: .milliseconds(100))
        }
        if let request = pending.first {
            #expect(request.name == "client")
            try await server.approve(id: request.id)
        } else {
            try await client.cancelPair()
            _ = try? await pairing.value
            throw ForwirohError("Pair request did not reach the local server")
        }
        let peer = try await pairing.value
        #expect(try await client.status().peers.first?.id == peer)
        #expect(try await client.status().peers.first?.policy.restricted == true)
        #expect(try await server.status().peers.first?.policy.services == ["pmai-acp": "127.0.0.1:19283"])
        let forward = try await client.forward(peer: peer, listen: "127.0.0.1:0", service: "127.0.0.1:22")
        let socks = try await client.socks(peer: peer, listen: "127.0.0.1:0")
        let tunnels = try await client.status().tunnels
        #expect(Set(tunnels.map(\.id)) == Set([forward, socks]))
        do {
            _ = try await client.command(["op": .string("socks_stop"), "id": .string(forward)])
            Issue.record("A mismatched stop must fail")
        } catch let error as ForwirohError {
            #expect(error.code == "wrong_tunnel_kind")
        }
        #expect(try await client.status().tunnels.contains(where: { $0.id == forward }))
        try await client.stopTunnel(id: forward)
        try await client.stopTunnel(id: TunnelID(rawValue: forward))
        try await client.stopTunnel(id: socks)
        try await client.stopTunnel(id: forward)
        try await client.stopTunnel(id: socks)
        try await client.stopTunnel(id: "missing")
        try await client.rename(peer: peer, name: "home")
        #expect(try await client.status().peers.first?.name == "home")
        try await client.setPolicy(peer: peer, policy: PeerPolicy(restricted: true))
        #expect(try await client.status().peers.first?.policy.restricted == true)
        let initialPolicy = try await client.peerPolicy(peer: peer)
        let updated = try await client.updatePolicy(peer: peer, expectedRevision: initialPolicy.revision, patch: ["services": .object(["ssh": .string("127.0.0.1:22")])])
        #expect(updated.policy.restricted && updated.policy.services["ssh"] == "127.0.0.1:22")
        do {
            _ = try await client.updatePolicy(peer: peer, expectedRevision: initialPolicy.revision, patch: ["restricted": .bool(false)])
            Issue.record("A stale revision must fail")
        } catch let error as ForwirohError { #expect(error.code == "policy_conflict") }
        try await client.revoke(peer: peer)
        #expect(try await client.status().peers.isEmpty)
    } catch {
        await client.close()
        await server.close()
        throw error
    }
    await client.close()
    await server.close()
}
#endif
