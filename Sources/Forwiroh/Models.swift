import Foundation

public struct ForwirohError: Error, LocalizedError, Sendable, Equatable {
    public let message: String
    /// Stable core error code; nil for older cores and local validation errors.
    public let code: String?
    public init(_ message: String, code: String? = nil) {
        self.message = message
        self.code = code
    }
    /// Preserve existing message-based equality; compare `code` explicitly.
    public static func == (lhs: Self, rhs: Self) -> Bool { lhs.message == rhs.message }
    public var errorDescription: String? { message }
}

/// A Sendable escape hatch for new commands and event fields in the core protocol.
public enum JSONValue: Codable, Sendable, Equatable {
    case null, bool(Bool), integer(Int64), number(Double), string(String)
    case array([JSONValue]), object([String: JSONValue])

    public init(from decoder: any Decoder) throws {
        let value = try decoder.singleValueContainer()
        if value.decodeNil() { self = .null }
        else if let v = try? value.decode(Bool.self) { self = .bool(v) }
        else if let v = try? value.decode(Int64.self) { self = .integer(v) }
        else if let v = try? value.decode(Double.self) { self = .number(v) }
        else if let v = try? value.decode(String.self) { self = .string(v) }
        else if let v = try? value.decode([JSONValue].self) { self = .array(v) }
        else { self = .object(try value.decode([String: JSONValue].self)) }
    }
    public func encode(to encoder: any Encoder) throws {
        var value = encoder.singleValueContainer()
        switch self {
        case .null: try value.encodeNil()
        case .bool(let v): try value.encode(v)
        case .integer(let v): try value.encode(v)
        case .number(let v): try value.encode(v)
        case .string(let v): try value.encode(v)
        case .array(let v): try value.encode(v)
        case .object(let v): try value.encode(v)
        }
    }
    public subscript(key: String) -> JSONValue? {
        guard case .object(let value) = self else { return nil }
        return value[key]
    }
    public var string: String? {
        guard case .string(let value) = self else { return nil }
        return value
    }
    func decode<T: Decodable>(_ type: T.Type) throws -> T {
        try JSONDecoder().decode(type, from: JSONEncoder().encode(self))
    }
}

public struct APIInfo: Decodable, Sendable {
    public let controlAPIVersion: UInt32
    public let cABIVersion: UInt32
    public let version: String
    public let commands: [String]
    public let aliases: [String: String]
    public let backends: [String]
    public let features: [String]
    public let maxPacketSize: Int
    public let policySchemaVersion: UInt32?
    public func supports(_ command: String) -> Bool {
        commands.contains(command) || aliases[command] != nil || (command == "stop" && features.contains("legacy_stop"))
    }
    enum CodingKeys: String, CodingKey {
        case version, commands, aliases, backends, features
        case controlAPIVersion = "control_api_version", cABIVersion = "c_abi_version", maxPacketSize = "max_packet_size"
        case policySchemaVersion = "policy_schema_version"
    }
}

public struct Status: Decodable, Sendable {
    public let id: String
    public let name: String
    public let version: String
    public let peers: [Peer]
    public let tunnels: [Tunnel]
}

public struct Peer: Decodable, Sendable, Identifiable {
    public let id: String
    public let name: String
    public let device: String
    public let connected: Bool
    public let path: String?
    public let policy: PeerPolicy
    public let policyRevision: String?
    enum CodingKeys: String, CodingKey { case id, name, device, connected, path, policy; case policyRevision = "policy_revision" }
}

public struct PolicySnapshot: Decodable, Sendable {
    public let policy: PeerPolicy
    public let revision: String
    public let policySchemaVersion: UInt32
    enum CodingKeys: String, CodingKey { case policy, revision; case policySchemaVersion = "policy_schema_version" }
}

public struct Tunnel: Decodable, Sendable, Identifiable {
    public let id: String
    public var tunnelID: TunnelID { TunnelID(rawValue: id) }
    public let sessionID: VPNSessionID?
    public let peer: String
    public let kind: String
    public let detail: String
    enum CodingKeys: String, CodingKey { case id, peer, kind, detail; case sessionID = "session_id" }
}

public struct Invitation: Decodable, Sendable, Identifiable {
    public let id: String
    public let uri: String
    /// Unix timestamp in seconds.
    public let expires: UInt64
}

public struct PairingRequest: Decodable, Sendable, Identifiable {
    public let id: String
    public let peer: String
    public let name: String
    public let device: String
}

/// Policies apply to incoming requests from this peer. Pairing defaults to open access.
public struct PeerPolicy: Codable, Sendable, Equatable {
    public var restricted: Bool
    public var services: [String: String]
    public var socksNetworks: [String]
    public var socksPorts: [UInt16]
    public var reversePorts: [UInt16]
    public var vpn: VPNPolicy?
    private var additionalFields: [String: JSONValue] = [:]

    public init(restricted: Bool = false, services: [String: String] = [:],
                socksNetworks: [String] = [], socksPorts: [UInt16] = [],
                reversePorts: [UInt16] = [], vpn: VPNPolicy? = nil) {
        self.restricted = restricted
        self.services = services
        self.socksNetworks = socksNetworks
        self.socksPorts = socksPorts
        self.reversePorts = reversePorts
        self.vpn = vpn
    }
    enum CodingKeys: String, CodingKey {
        case restricted, services, vpn
        case socksNetworks = "socks_networks", socksPorts = "socks_ports", reversePorts = "reverse_ports"
    }
    public init(from decoder: any Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        restricted = try fields.decodeIfPresent(Bool.self, forKey: .restricted) ?? false
        services = try fields.decodeIfPresent([String: String].self, forKey: .services) ?? [:]
        socksNetworks = try fields.decodeIfPresent([String].self, forKey: .socksNetworks) ?? []
        socksPorts = try fields.decodeIfPresent([UInt16].self, forKey: .socksPorts) ?? []
        reversePorts = try fields.decodeIfPresent([UInt16].self, forKey: .reversePorts) ?? []
        vpn = try fields.decodeIfPresent(VPNPolicy.self, forKey: .vpn)
        additionalFields = try [String: JSONValue](from: decoder).filter { CodingKeys(rawValue: $0.key) == nil }
    }
    public func encode(to encoder: any Encoder) throws {
        var fields = additionalFields
        fields["restricted"] = .bool(restricted)
        fields["services"] = .object(services.mapValues(JSONValue.string))
        fields["socks_networks"] = .array(socksNetworks.map(JSONValue.string))
        fields["socks_ports"] = .array(socksPorts.map { .integer(Int64($0)) })
        fields["reverse_ports"] = .array(reversePorts.map { .integer(Int64($0)) })
        if let vpn { fields["vpn"] = try JSONDecoder().decode(JSONValue.self, from: JSONEncoder().encode(vpn)) }
        try fields.encode(to: encoder)
    }
}

public struct VPNPolicy: Codable, Sendable, Equatable {
    public var routes: [String]
    public var localIP: String
    public var peerIP: String
    public var tun: String
    private var additionalFields: [String: JSONValue] = [:]
    public init(routes: [String], localIP: String, peerIP: String, tun: String = "") {
        self.routes = routes; self.localIP = localIP; self.peerIP = peerIP; self.tun = tun
    }
    enum CodingKeys: String, CodingKey {
        case routes, tun
        case localIP = "local_ip", peerIP = "peer_ip"
    }
    public init(from decoder: any Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        routes = try fields.decode([String].self, forKey: .routes)
        localIP = try fields.decode(String.self, forKey: .localIP)
        peerIP = try fields.decode(String.self, forKey: .peerIP)
        tun = try fields.decodeIfPresent(String.self, forKey: .tun) ?? ""
        additionalFields = try [String: JSONValue](from: decoder).filter { CodingKeys(rawValue: $0.key) == nil }
    }
    public func encode(to encoder: any Encoder) throws {
        var fields = additionalFields
        fields["routes"] = .array(routes.map(JSONValue.string))
        fields["local_ip"] = .string(localIP); fields["peer_ip"] = .string(peerIP); fields["tun"] = .string(tun)
        try fields.encode(to: encoder)
    }
}

public struct VPNSession: Decodable, Sendable, Identifiable {
    public var id: String { sessionID.rawValue }
    public let sessionID: VPNSessionID
    public let tunnelID: TunnelID?
    public let localIP: String
    public let remoteIP: String
    public let localIPv6: String
    public let remoteIPv6: String
    public let forwardIP: String
    public let routes: [String]
    public let mtu: Int
    enum CodingKeys: String, CodingKey {
        case id, routes, mtu
        case sessionID = "session_id", tunnelID = "tunnel_id"
        case localIP = "local_ip", remoteIP = "remote_ip"
        case localIPv6 = "local_ipv6", remoteIPv6 = "remote_ipv6", forwardIP = "forward_ip"
    }
    public init(from decoder: any Decoder) throws {
        let fields = try decoder.container(keyedBy: CodingKeys.self)
        let legacy = try fields.decodeIfPresent(String.self, forKey: .id)
        let explicit = try fields.decodeIfPresent(VPNSessionID.self, forKey: .sessionID)
        guard let id = explicit?.rawValue ?? legacy else {
            throw DecodingError.keyNotFound(CodingKeys.id, .init(codingPath: decoder.codingPath, debugDescription: "Missing VPN session ID"))
        }
        guard legacy == nil || legacy == id else {
            throw DecodingError.dataCorruptedError(forKey: .id, in: fields, debugDescription: "Conflicting VPN session IDs")
        }
        sessionID = VPNSessionID(rawValue: id)
        tunnelID = try fields.decodeIfPresent(TunnelID.self, forKey: .tunnelID)
        localIP = try fields.decode(String.self, forKey: .localIP)
        remoteIP = try fields.decode(String.self, forKey: .remoteIP)
        localIPv6 = try fields.decode(String.self, forKey: .localIPv6)
        remoteIPv6 = try fields.decode(String.self, forKey: .remoteIPv6)
        forwardIP = try fields.decode(String.self, forKey: .forwardIP)
        routes = try fields.decode([String].self, forKey: .routes)
        mtu = try fields.decode(Int.self, forKey: .mtu)
    }
}

public struct QRCode: Decodable, Sendable {
    public let width: Int
    /// Row-major matrix; true is a dark module. Add a quiet zone when drawing.
    public let pixels: [Bool]
}

public struct CoreEvent: Sendable {
    public let name: String
    public let payload: JSONValue
}
