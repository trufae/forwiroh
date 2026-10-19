import NetworkExtension
import Foundation
import os

/// Mutable provider state is confined to the serial work queue.
final class PacketTunnelProvider: NEPacketTunnelProvider, @unchecked Sendable {
    private let work = DispatchQueue(label: "org.radare.forwiroh.vpn")
    private let log = Logger(subsystem: "org.radare.forwiroh", category: "VPN")
    private var core: RustCore?
    private var sessionID: String?
    private var activeForwards: [String: [String: Any]] = [:]
    private var events: [[String: Any]] = []
    private var addresses: [String: Any] = [:]

    override func startTunnel(options: [String: NSObject]?, completionHandler: @escaping @Sendable (Error?) -> Void) {
        work.async { [self] in
            RustCore.saveVPNError(nil)
            do {
                guard let config = (protocolConfiguration as? NETunnelProviderProtocol)?.providerConfiguration,
                      let peer = config["peer"] as? String, let routes = config["routes"] as? [String] else {
                    throw ForwirohError(message: "Missing VPN peer or routes")
                }
                log.info("Starting VPN to \(peer, privacy: .public)")
                let core = try RustCore(); self.core = core
                core.events { [weak self] data in
                    self?.work.async { [weak self] in
                        guard let self,
                              let event = try? JSONSerialization.jsonObject(with: data) as? [String: Any] else { return }
                        if let error = event["error"] as? String { self.log.error("\(error, privacy: .public)") }
                        if event["event"] as? String != "access_request" {
                            self.events.append(event)
                            if self.events.count > 32 { self.events.removeFirst() }
                        }
                    }
                }
                let info = try core.command(["op": "vpn_start", "peer": peer, "routes": routes])
                guard let id = info["id"] as? String, let local = info["local_ip"] as? String,
                      let remote = info["remote_ip"] as? String, let mtu = info["mtu"] as? NSNumber else {
                    throw ForwirohError(message: "Invalid VPN negotiation; update the server")
                }
                sessionID = id; addresses = info
                let settings = NEPacketTunnelNetworkSettings(tunnelRemoteAddress: remote)
                let ipv4 = NEIPv4Settings(addresses: [local], subnetMasks: ["255.255.255.255"])
                ipv4.includedRoutes = try routes.filter { !$0.contains(":") }.map(Self.route)
                settings.ipv4Settings = ipv4
                if let local6 = info["local_ipv6"] as? String, routes.contains(where: { $0.contains(":") }) {
                    let ipv6 = NEIPv6Settings(addresses: [local6], networkPrefixLengths: [128])
                    ipv6.includedRoutes = try routes.filter { $0.contains(":") }.map { cidr in
                        let parts = cidr.split(separator: "/")
                        guard parts.count == 2, let bits = Int(parts[1]), (0...128).contains(bits) else {
                            throw ForwirohError(message: "Invalid IPv6 route: \(cidr)")
                        }
                        return NEIPv6Route(destinationAddress: String(parts[0]), networkPrefixLength: NSNumber(value: bits))
                    }
                    settings.ipv6Settings = ipv6
                }
                // NECP bypasses the VPN for sockets opened by this provider process,
                // including Iroh's BSD UDP and relay sockets. Do not exclude peer IPs:
                // the same LAN host may also be a requested VPN destination.
                settings.mtu = mtu
                if let dns = config["dns"] as? String, !dns.isEmpty {
                    guard routes.contains(where: { Self.contains($0, dns) }) else {
                        throw ForwirohError(message: "DNS server must be inside a VPN route")
                    }
                    let dnsSettings = NEDNSSettings(servers: [dns]); dnsSettings.matchDomains = [""]
                    settings.dnsSettings = dnsSettings
                }
                // Recreate the app's listeners in the extension before reporting connected.
                // Starting VPN resets existing TCP streams during the ownership handoff.
                activeForwards.removeAll()
                for command in RustCore.savedForwards {
                    let result = try core.command(command)
                    if let id = result["id"] as? String { activeForwards[id] = command }
                }
                setTunnelNetworkSettings(settings) { [weak self] error in
                    guard let self else { return }
                    if let error {
                        self.work.async { self.fail(error); completionHandler(error) }
                        return
                    }
                    self.readPackets(core: core, id: id)
                    DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                        while let packet = core.receivePacket(session: id) {
                            let proto = (packet.first ?? 0) >> 4 == 6 ? AF_INET6 : AF_INET
                            self?.packetFlow.writePackets([packet], withProtocols: [NSNumber(value: proto)])
                        }
                        self?.work.async { [weak self] in
                            guard let self, self.sessionID == id else { return }
                            let error = ForwirohError(message: "VPN peer disconnected. Reconnect to resume forwarding.")
                            self.fail(error); self.cancelTunnelWithError(error)
                        }
                    }
                    self.log.info("VPN connected; forwarded ports at 10.250.0.3")
                    completionHandler(nil)
                }
            } catch {
                fail(error); completionHandler(error)
            }
        }
    }
    private func fail(_ error: Error) {
        RustCore.saveVPNError(error.localizedDescription)
        log.error("VPN failed: \(error.localizedDescription, privacy: .public)")
        sessionID = nil
        core?.close(); core = nil
        activeForwards.removeAll()
    }
    private func readPackets(core: RustCore, id: String) {
        packetFlow.readPackets { [weak self] packets, protocols in
            guard let self else { return }
            self.work.async {
                guard self.sessionID == id else { return }
                for (packet, proto) in zip(packets, protocols) where proto.int32Value == AF_INET || proto.int32Value == AF_INET6 {
                    _ = core.sendPacket(session: id, data: packet)
                }
                self.readPackets(core: core, id: id)
            }
        }
    }
    override func stopTunnel(with reason: NEProviderStopReason, completionHandler: @escaping @Sendable () -> Void) {
        work.async { [self] in
            sessionID = nil
            core?.close(); core = nil; activeForwards.removeAll()
            log.info("VPN stopped (reason \(reason.rawValue))")
            setTunnelNetworkSettings(nil) { _ in completionHandler() }
        }
    }
    override func handleAppMessage(_ messageData: Data, completionHandler: (@Sendable (Data?) -> Void)?) {
        work.async { [self] in
            guard let core, let id = sessionID else {
                completionHandler?(Self.response(error: "VPN is not connected")); return
            }
            guard messageData.count < 65_536, (try? JSONSerialization.jsonObject(with: messageData)) is [String: Any] else {
                completionHandler?(Self.response(error: "Invalid VPN command")); return
            }
            // Commands such as reverse/pair can wait for remote approval. They must
            // never block packet processing, status, cancellation or shutdown.
            DispatchQueue.global(qos: .userInitiated).async { [weak self] in
                let result = Result { try core.command(messageData) }
                self?.work.async { [weak self] in
                    guard let self, self.core === core, self.sessionID == id else {
                        completionHandler?(Self.response(error: "VPN stopped")); return
                    }
                    do {
                        var value = try RustCore.decodeResponse(result.get())
                        guard let command = try JSONSerialization.jsonObject(with: messageData) as? [String: Any] else {
                            throw ForwirohError(message: "Invalid VPN command")
                        }
                        let op = command["op"] as? String ?? ""
                        if ["forward", "reverse", "socks"].contains(op), let forward = value["id"] as? String {
                            self.activeForwards[forward] = command
                        }
                        if op == "stop", let forward = command["id"] as? String { self.activeForwards.removeValue(forKey: forward) }
                        // Only persist currently active listeners. Disconnect/revoke can
                        // remove several at once, including listeners on another peer.
                        if let status = try? core.command(["op": "status"]), let tunnels = status["tunnels"] as? [[String: Any]] {
                            let ids = Set(tunnels.compactMap { $0["id"] as? String })
                            self.activeForwards = self.activeForwards.filter { ids.contains($0.key) }
                        }
                        if op == "status" {
                            value["traffic"] = try? core.command(["op": "vpn_stats", "id": id])
                            value["vpn"] = self.addresses
                            value["events"] = self.events; self.events.removeAll()
                        }
                        do { try RustCore.saveForwards(Array(self.activeForwards.values)) }
                        catch { self.log.error("Cannot save forward settings: \(error.localizedDescription, privacy: .public)") }
                        completionHandler?(try? JSONSerialization.data(withJSONObject: ["data": value]))
                    } catch { completionHandler?(Self.response(error: error.localizedDescription)) }
                }
            }
        }
    }
    private static func response(error: String) -> Data? {
        try? JSONSerialization.data(withJSONObject: ["error": error])
    }
    private static func ip(_ value: String) -> UInt32? {
        let parts = value.split(separator: "."); guard parts.count == 4 else { return nil }
        var result: UInt32 = 0
        for p in parts { guard let byte = UInt8(p) else { return nil }; result = result << 8 | UInt32(byte) }
        return result
    }
    private static func contains(_ cidr: String, _ address: String) -> Bool {
        let parts = cidr.split(separator: "/")
        guard parts.count == 2, let bits = Int(parts[1]), (0...32).contains(bits),
              let net = ip(String(parts[0])), let addr = ip(address) else { return false }
        let mask = bits == 0 ? 0 : UInt32.max << (32 - bits)
        return net & mask == addr & mask
    }
    private static func route(_ cidr: String) throws -> NEIPv4Route {
        let parts = cidr.split(separator: "/")
        guard parts.count == 2, let bits = Int(parts[1]), (0...32).contains(bits), let net = ip(String(parts[0])) else {
            throw ForwirohError(message: "Invalid IPv4 route: \(cidr)")
        }
        let mask = bits == 0 ? 0 : UInt32.max << (32 - bits)
        func dotted(_ value: UInt32) -> String { [24, 16, 8, 0].map { String((value >> $0) & 255) }.joined(separator: ".") }
        return NEIPv4Route(destinationAddress: dotted(net & mask), subnetMask: dotted(mask))
    }
}
