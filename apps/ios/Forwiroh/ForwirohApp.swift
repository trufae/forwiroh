import SwiftUI
@main
struct ForwirohApp: App {
    @StateObject private var model = Model()
    @Environment(\.scenePhase) private var scenePhase
    var body: some Scene {
        WindowGroup {
            PeersView().environmentObject(model).task { await model.start() }
                .onChange(of: scenePhase) { _, phase in
                    Task {
                        if phase == .background { await model.suspend() }
                        else if phase == .active { await model.start() }
                    }
                }
                .alert("Forwiroh", isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })) {
                    Button("OK") { model.error = nil }
                } message: { Text(model.error ?? "") }
        }
    }
}
struct PeersView: View {
    @EnvironmentObject var model: Model
    @State private var add = false
    private var appVersion: String {
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "Unknown"
        let build = Bundle.main.object(forInfoDictionaryKey: "CFBundleVersion") as? String ?? "Unknown"
        return "\(version) (build \(build))"
    }
    var body: some View {
        NavigationStack {
            List {
                if model.vpnRunning {
                    Section("VPN") {
                        Label(model.vpnStatusText, systemImage: "network.badge.shield.half.filled")
                        Button("Disconnect VPN") { model.stopVPN() }
                        Button("Transfer status") { model.refreshVPNStats() }
                        if !model.vpnStats.isEmpty { Text(model.vpnStats).font(.caption.monospaced()) }; if !model.vpnHost.isEmpty { Text(model.vpnHost).font(.caption.monospaced()).textSelection(.enabled) }
                    }
                }
                Section("Paired devices") {
                    if model.peers.isEmpty { Text("Run forwiroh serve and forwiroh pair on your computer, then scan its QR code.").foregroundStyle(.secondary) }
                    ForEach(model.peers) { peer in
                        NavigationLink { PeerView(peerID: peer.id) } label: {
                            VStack(alignment: .leading) {
                                Label(peer.name, systemImage: peer.device == "iOS" ? "iphone" : "desktopcomputer")
                                Text(peer.connected ? "Connected · \(peer.path ?? "connecting")" : "Disconnected").font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                Section {
                    Text("Enable VPN to keep port forwarding and SOCKS active when switching apps. VPN can route only your forwarded ports or all internet traffic.").font(.footnote).foregroundStyle(.secondary)
                }
                Section("Tools") {
                    NavigationLink("TCP test client") { TCPClientView() }
                }
                Section("About Forwiroh") {
                    LabeledContent("App version", value: appVersion)
                    LabeledContent("Rust core version", value: RustCore.version)
                    NavigationLink("Crypto information") { CryptoInfoView() }
                }
            }
            .navigationTitle("Forwiroh")
            .toolbar { Button { add = true } label: { Image(systemName: "qrcode.viewfinder") }.disabled(!model.ready) }
            .sheet(isPresented: $add) { AddPeerView() }
        }
        .overlay(alignment: .bottom) {
            if let notice = model.notice {
                Label(notice, systemImage: "checkmark.circle.fill")
                    .font(.callout).padding()
                    .background(.regularMaterial, in: Capsule()).padding()
                    .transition(.move(edge: .bottom).combined(with: .opacity))
            }
        }
        .animation(.easeInOut, value: model.notice)
        .task(id: model.notice) {
            guard let notice = model.notice else { return }
            try? await Task.sleep(for: .seconds(3))
            if !Task.isCancelled && model.notice == notice { model.notice = nil }
        }
    }
}
struct AddPeerView: View {
    @EnvironmentObject var model: Model
    @Environment(\.dismiss) var dismiss
    @State private var scan = true
    @State private var uri = ""
    @State private var confirm = false
    var body: some View {
        NavigationStack {
            VStack(spacing: 20) {
                if model.pairing {
                    ProgressView("Waiting for approval on the computer…")
                    Button("Cancel pairing", role: .cancel) { Task { await model.run(["op": "cancel_pair"]) } }
                }
                else if scan {
                    ScannerView { result in
                        switch result {
                        case .success(let value): uri = value; scan = false; confirm = true
                        case .failure(let error): model.error = error.localizedDescription; scan = false
                        }
                    }.clipShape(RoundedRectangle(cornerRadius: 20))
                    Text("Scan the QR printed by forwiroh pair.").foregroundStyle(.secondary)
                } else {
                    TextField("forwiroh://pair?data=…", text: $uri, axis: .vertical).textInputAutocapitalization(.never).autocorrectionDisabled()
                    Button("Review invitation") { confirm = true }.disabled(!uri.hasPrefix("forwiroh://pair?data="))
                }
                Button(scan ? "Paste invitation instead" : "Scan QR") { scan.toggle() }.disabled(model.pairing)
            }.padding().navigationTitle("Add peer")
            .toolbar { Button("Close") { dismiss() }.disabled(model.pairing) }
            .confirmationDialog("Pair with this device?", isPresented: $confirm, titleVisibility: .visible) {
                Button("Pair") { Task { await model.pair(uri); if model.error == nil { dismiss() } } }
            } message: { Text(invitationSummary) }
        }
    }
    private var invitationSummary: String {
        // Display only; authoritative bounded parsing/expiry checking is in Rust.
        guard uri.count < 4096, let data = uri.components(separatedBy: "?data=").last else { return "Invalid invitation" }
        let normalized = data.replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
        let padded = normalized + String(repeating: "=", count: (4 - normalized.count % 4) % 4)
        guard let decoded = Data(base64Encoded: padded), let value = try? JSONSerialization.jsonObject(with: decoded) as? [String: Any] else { return "The core will validate this invitation." }
        let endpoint = (value["addr"] as? [String: Any])?["id"] as? String ?? "Unknown identity"
        return "\(value["name"] as? String ?? "Device")\n\(endpoint)\nCheck the requesting identity on the computer before approving."
    }
}
struct PeerView: View {
    @EnvironmentObject var model: Model
    let peerID: String
    @State private var services: [String] = []
    @State private var remoteRestricted = true
    @State private var connecting = false
    @State private var rename = ""
    @State private var policy = ""
    @State private var showPolicy = false
    @State private var allowAll = true
    @State private var remove = false
    var peer: Peer? { model.peers.first { $0.id == peerID } }
    var body: some View {
        List {
            if let peer {
                Section("Connection") {
                    Text(peer.id).font(.caption.monospaced()).textSelection(.enabled)
                    Text(connecting ? "Connecting…" : peer.connected ? "Connected · \(peer.path ?? "unknown")" : "Disconnected")
                    Button { Task { await connect(showFeedback: true) } } label: {
                        HStack {
                            if connecting { ProgressView() }
                            Text(connecting ? "Connecting…" : peer.connected ? "Refresh connection" : "Connect")
                        }
                    }.disabled(!model.ready || connecting)
                    Button("Disconnect") { Task { await model.run(["op": "disconnect", "peer": peerID], success: "Disconnected from \(peer.name)") } }.disabled(!model.ready || connecting)
                }
                Section("Services") {
                    Text(remoteRestricted ? "\(peer.name) accepts its configured services and ports." : "Choose any destination host and port on \(peer.name). Paired requests are accepted and logged.").font(.footnote)
                    ForEach(services, id: \.self) { Text($0) }
                    NavigationLink("Port forwarding") { ForwardView(peer: peerID, services: services, restricted: remoteRestricted) }.disabled(!model.ready || connecting)
                    NavigationLink("SOCKS5 proxy") { SocksView(peer: peerID) }.disabled(!model.ready || connecting)
                    NavigationLink("VPN") { VPNView(peer: peerID) }
                }
                Section("Active tunnels") {
                    NavigationLink("Test a TCP connection") { TCPClientView() }
                    ForEach(model.tunnels.filter { $0.peer == peerID }) { t in
                        VStack(alignment: .leading) {
                            Text("\(t.kind): \(t.detail)").font(.caption)
                            Button("Stop", role: .destructive) { if t.kind == "vpn" { model.stopVPN() } else { Task { await model.run(["op": "stop", "id": t.id]) } } }
                        }
                    }
                }
                Section("Device") {
                    TextField("New name", text: $rename)
                    Button("Rename") { Task { await model.run(["op": "rename", "peer": peerID, "name": rename]) } }.disabled(rename.isEmpty || !model.ready)
                    Button("Access settings") {
                        policy = (try? String(data: JSONSerialization.data(withJSONObject: peer.policy, options: [.prettyPrinted, .sortedKeys]), encoding: .utf8)) ?? "{}"
                        allowAll = !(peer.policy["restricted"] as? Bool ?? false)
                        showPolicy = true
                    }.disabled(!model.ready)
                    Button("Remove and revoke", role: .destructive) { remove = true }.disabled(!model.ready)
                }
            } else { Text("Peer removed") }
        }.navigationTitle(peer?.name ?? "Peer")
        .task { if model.ready { await connect() } }
        .confirmationDialog("Revoke this device and close its sessions?", isPresented: $remove) {
            Button("Revoke", role: .destructive) { Task { await model.run(["op": "revoke", "peer": peerID]) } }
        }
        .sheet(isPresented: $showPolicy) {
            NavigationStack {
                Form {
                    Toggle("Allow connections from this paired peer", isOn: $allowAll)
                    Text("This controls access to this iPhone. Paired TCP, UDP VPN, SOCKS and reverse-forward requests are accepted and logged by default. Turn this off to use the service, port and VPN route restrictions below.").font(.footnote)
                    DisclosureGroup("Service, port and VPN settings") {
                        TextEditor(text: $policy).font(.caption.monospaced()).frame(minHeight: 260)
                    }
                    Button("Save access settings") {
                        Task {
                            do {
                                guard var value = try JSONSerialization.jsonObject(with: Data(policy.utf8)) as? [String: Any] else { throw ForwirohError(message: "Settings must be a JSON object") }
                                value["restricted"] = !allowAll
                                if await model.run(["op": "policy", "peer": peerID, "policy": value], success: "Access settings saved") { showPolicy = false }
                            } catch { model.error = error.localizedDescription }
                        }
                    }
                }.navigationTitle("Access settings").toolbar { Button("Close") { showPolicy = false } }
            }
        }
    }
    func connect(showFeedback: Bool = false) async {
        guard !connecting else { return }
        connecting = true; defer { connecting = false }
        model.error = nil
        do {
            let result = try await model.request(["op": "services", "peer": peerID])
            services = result["services"] as? [String] ?? []
            remoteRestricted = result["restricted"] as? Bool ?? true
            await model.refresh()
            if showFeedback { model.notice = "Connected to \(peer?.name ?? "peer")" }
        }
        catch { model.error = error.localizedDescription }
    }
}
struct ForwardView: View {
    @EnvironmentObject var model: Model
    @Environment(\.dismiss) private var dismiss
    let peer: String
    let services: [String]
    let restricted: Bool
    @State private var reverse = false
    @State private var listenAll = false
    @State private var port = "2222"
    @State private var service = "ssh"
    @State private var targetHost = "localhost"
    @State private var targetPort = "22"
    @State private var starting = false
    var peerName: String { model.peers.first { $0.id == peer }?.name ?? "Remote device" }
    var listenerName: String { reverse ? peerName : "iPhone" }
    var listenAddress: String { !reverse && listenAll ? "0.0.0.0" : "127.0.0.1" }
    var listenerHost: String { !reverse && listenAll ? "0.0.0.0" : "localhost" }
    var destinationName: String { reverse ? "iPhone" : peerName }
    var destination: String {
        let host = reverse ? "localhost" : targetHost.trimmingCharacters(in: .whitespacesAndNewlines)
        return "\(host.contains(":") && !host.hasPrefix("[") ? "[\(host)]" : host):\(targetPort)"
    }
    var connectionPath: String { "\(listenerName) \(listenerHost):\(port) → \(destinationName) \(!reverse && restricted ? service : destination)" }
    var body: some View {
        Form {
            Section("Connection path") {
                Text(connectionPath).font(.callout.monospaced()).textSelection(.enabled)
                Text("Connections arriving on \(listenerName) are forwarded to the destination on \(destinationName).").font(.footnote)
            }
            Section("Listen on") {
                Picker("Device", selection: $reverse) { Text("iPhone").tag(false); Text(peerName).tag(true) }.pickerStyle(.segmented)
                if !reverse {
                    Toggle("Listen on all interfaces (0.0.0.0)", isOn: $listenAll)
                    LabeledContent("Listening address", value: listenAddress)
                }
                LabeledContent("Listening port") { TextField("2222", text: $port).keyboardType(.numberPad).multilineTextAlignment(.trailing) }
            }
            Section("Connect to \(destinationName)") {
                if !reverse && restricted {
                    TextField("Saved service name", text: $service).textInputAutocapitalization(.never).autocorrectionDisabled()
                    if !services.isEmpty { Picker("Saved services", selection: $service) { ForEach(services, id: \.self) { Text($0) } } }
                    Text("\(peerName) has enabled access restrictions. Select a service configured there.").font(.footnote)
                } else {
                    if reverse { LabeledContent("Destination host", value: "localhost") }
                    else { TextField("Destination host on \(peerName)", text: $targetHost).textInputAutocapitalization(.never).autocorrectionDisabled() }
                    LabeledContent("Destination port") { TextField("22", text: $targetPort).keyboardType(.numberPad).multilineTextAlignment(.trailing) }
                }
            }
            Button {
                starting = true
                Task {
                    defer { starting = false }
                    guard let p = UInt16(port), p > 0 else { model.error = "Enter a port between 1 and 65535"; return }
                    let command: [String: Any]
                    if reverse {
                        guard let target = UInt16(targetPort), target > 0 else { model.error = "Invalid destination port"; return }
                        command = ["op": "reverse", "peer": peer, "port": Int(p), "target": "127.0.0.1:\(target)"]
                    } else {
                        if restricted {
                            guard !service.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty else { model.error = "Enter a saved service name"; return }
                        } else {
                            guard let target = UInt16(targetPort), target > 0 else { model.error = "Enter a destination port between 1 and 65535"; return }
                            let host = targetHost.trimmingCharacters(in: .whitespacesAndNewlines)
                            guard !host.isEmpty && host.count <= 253 && !host.contains(where: { $0.isWhitespace }) else { model.error = "Enter a destination hostname or IP address"; return }
                        }
                        command = ["op": "forward", "peer": peer, "listen": "\(listenAddress):\(p)", "service": restricted ? service : destination]
                    }
                    if await model.run(command, success: "Forwarding started on \(listenerName) \(listenerHost):\(p)") { dismiss() }
                }
            } label: { HStack { if starting { ProgressView() }; Text(starting ? "Starting forwarding…" : "Start forwarding") } }
            Text(!reverse && listenAll ? "From your Mac or another device on the same Wi-Fi, connect to this iPhone's Wi-Fi IP address on port \(port). Enable VPN to keep it running when switching apps." : "Connect to localhost:\(port) on \(listenerName) to use this forward. Enable VPN to keep it running when switching apps.").font(.footnote)
            if model.vpnStatus == .connected && !reverse {
                Text("Also connect to 10.250.0.3:\(port) from any app on this iPhone. The VPN extension keeps this forward alive in the background.").font(.footnote)
            }
        }.navigationTitle("Port forwarding").disabled(starting || !model.ready).navigationBarBackButtonHidden(starting)
    }
}
struct SocksView: View {
    @EnvironmentObject var model: Model
    @Environment(\.dismiss) private var dismiss
    let peer: String
    @State private var port = "1080"
    @State private var starting = false
    var body: some View {
        Form {
            TextField("Listening port on iPhone", text: $port).keyboardType(.numberPad)
            Button {
                starting = true
                Task {
                    defer { starting = false }
                    guard let p = UInt16(port), p > 0 else { model.error = "Invalid port"; return }
                    if await model.run(["op": "socks", "peer": peer, "listen": "127.0.0.1:\(p)"], success: "SOCKS5 listening on iPhone:\(p)") { dismiss() }
                }
            } label: { HStack { if starting { ProgressView() }; Text(starting ? "Starting SOCKS5…" : "Start SOCKS5") } }
            Text("Configure your client to use iPhone localhost:\(port). The paired device opens the requested destination and resolves its hostname. Requests are accepted and logged unless that device enables access restrictions.").font(.footnote)
        }.navigationTitle("SOCKS5").disabled(starting || !model.ready).navigationBarBackButtonHidden(starting)
    }
}
struct TCPClientView: View {
    @StateObject private var client = TCPClient()
    @Environment(\.scenePhase) private var scenePhase
    @State private var host = "127.0.0.1"
    @State private var port = "2222"
    @State private var message = ""
    @State private var appendNewline = true
    private var canSend: Bool { client.connected && !client.sending && (!message.isEmpty || appendNewline) }
    private func send() {
        guard canSend else { return }
        client.send(message + (appendNewline ? "\r\n" : ""))
        message = ""
    }
    var body: some View {
        VStack(spacing: 0) {
            VStack(alignment: .leading, spacing: 10) {
                HStack {
                    TextField("Host", text: $host).textInputAutocapitalization(.never).autocorrectionDisabled().accessibilityLabel("Destination host")
                    TextField("Port", text: $port).keyboardType(.numberPad).frame(width: 75).accessibilityLabel("Destination port")
                }.textFieldStyle(.roundedBorder).disabled(client.connecting || client.connected)
                HStack {
                    if client.connecting { ProgressView().controlSize(.small) }
                    Text(client.status).font(.caption).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
                    if client.connecting || client.connected { Button("Disconnect") { client.disconnect() } }
                    else { Button("Connect") { client.connect(host: host, port: port) } }
                }.buttonStyle(.bordered)
            }.padding()
            Divider()
            ScrollViewReader { scroll in
                ScrollView {
                    VStack(alignment: .leading, spacing: 0) {
                        Text(client.output.isEmpty ? "Connect to 127.0.0.1:2222 to test your forward. An SSH destination should send an SSH-2.0 banner." : client.output)
                            .font(.system(.body, design: .monospaced)).textSelection(.enabled)
                            .foregroundStyle(client.output.isEmpty ? .secondary : .primary)
                            .frame(maxWidth: .infinity, alignment: .leading)
                        Color.clear.frame(height: 1).id("end")
                    }.padding()
                }
                .scrollDismissesKeyboard(.interactively)
                .frame(maxWidth: .infinity, maxHeight: .infinity)
                .onChange(of: client.output) { _, _ in scroll.scrollTo("end", anchor: .bottom) }
            }
        }.navigationTitle("TCP test client")
            .safeAreaInset(edge: .bottom, spacing: 0) {
                VStack(spacing: 10) {
                    Divider()
                    HStack(alignment: .bottom) {
                        TextField("Text to send", text: $message, axis: .vertical)
                            .lineLimit(1...5).textFieldStyle(.roundedBorder)
                            .textInputAutocapitalization(.never).autocorrectionDisabled()
                            .submitLabel(.send).onSubmit { send() }
                        Button(action: send) {
                            if client.sending { ProgressView() }
                            else { Image(systemName: "arrow.up").font(.headline) }
                        }.buttonStyle(.borderedProminent).disabled(!canSend).accessibilityLabel("Send text")
                    }.padding(.horizontal)
                    Toggle("Append newline (CRLF)", isOn: $appendNewline).font(.caption).padding(.horizontal)
                }.padding(.bottom, 10).background(Color(.secondarySystemBackground))
            }
            .toolbar { Button("Clear") { client.clear() } }
            .onDisappear { client.disconnect() }
            .onChange(of: scenePhase) { _, phase in if phase == .background { client.disconnect() } }
    }
}

struct CryptoInfoView: View {
    private let info = RustCore.cryptoInfo
    var body: some View {
        List {
            Section("Identity") {
                LabeledContent("Signing algorithm", value: info["identity_algorithm"] as? String ?? "Unknown")
                LabeledContent("Public key", value: "\(info["public_key_bits"] as? Int ?? 0) bits (32 bytes)")
                LabeledContent("Private seed", value: "\(info["secret_seed_bits"] as? Int ?? 0) bits (32 bytes)")
                LabeledContent("Key storage", value: "Apple Keychain · this device only")
            }
            Section("Encrypted peer transport") {
                LabeledContent("Transport", value: info["transport"] as? String ?? "Unknown")
                LabeledContent("TLS", value: info["tls"] as? String ?? "Unknown")
                LabeledContent("Provider", value: info["provider"] as? String ?? "Unknown")
            }
            Section("Supported cipher suites") {
                ForEach(info["cipher_suites"] as? [String] ?? [], id: \.self) { Text($0).font(.caption.monospaced()).textSelection(.enabled) }
            }
            Section("Supported key exchanges") {
                ForEach(info["key_exchanges"] as? [String] ?? [], id: \.self) { Text($0).textSelection(.enabled) }
            }
            Section {
                Text("These are this build's configured capabilities. Each connection negotiates one cipher suite and key exchange. Key sizes above describe encoded lengths.").font(.footnote).foregroundStyle(.secondary)
            }
        }.navigationTitle("Crypto information")
    }
}

struct VPNView: View {
    @EnvironmentObject var model: Model
    @Environment(\.dismiss) private var dismiss
    let peer: String
    @State private var fullInternet = true
    @State private var route = ""
    @State private var dns = ""
    @State private var starting = false
    var routes: [String] {
        if fullInternet { return ["0.0.0.0/0", "::/0"] }
        return ["10.250.0.1/32", "10.250.0.3/32"] + route.split(separator: ",").map { $0.trimmingCharacters(in: .whitespaces) }
    }
    var body: some View {
        Form {
            Section {
                LabeledContent("Status", value: model.vpnStatusText)
                if model.vpnStatus == .connecting || starting { ProgressView("Starting VPN and restoring forwards…") }
                if let error = model.vpnError { Text(error).foregroundStyle(.red).textSelection(.enabled) }
            }
            Section("Traffic") {
                Toggle("Route all internet traffic", isOn: $fullInternet)
                Text(fullInternet ? "TCP and UDP traffic, including DNS, goes through this peer over IPv4 and IPv6. The server runs as an ordinary user." : "Only the virtual hosts and extra routes use VPN. Your other traffic uses the normal internet connection.").font(.footnote)
                if !fullInternet {
                    TextField("Extra routes, e.g. 192.168.1.0/24", text: $route).textInputAutocapitalization(.never).autocorrectionDisabled()
                }
                TextField(fullInternet ? "DNS server (default 1.1.1.1)" : "Optional DNS server inside a VPN route", text: $dns).keyboardType(.decimalPad)
            }
            Section("Port forwarding in the background") {
                Text("Existing forwards move into the VPN extension. New connections survive switching apps; existing streams reconnect once when VPN starts.").font(.footnote)
                Text("10.250.0.1:port → localhost:port on the server\n10.250.0.3:port → a forwarded port on this iPhone").font(.callout.monospaced()).textSelection(.enabled)
            }
            if model.vpnRunning { Button("Disconnect") { model.stopVPN() } }
            else {
                Button {
                    starting = true
                    Task {
                        await model.startVPN(peer: peer, routes: routes, dns: dns.isEmpty && fullInternet ? "1.1.1.1" : dns, fullInternet: fullInternet)
                        starting = false
                    }
                } label: { Text(starting ? "Starting VPN…" : "Connect VPN") }
                .disabled(starting || !model.ready)
            }
            if !model.vpnHost.isEmpty { Text(model.vpnHost).font(.caption.monospaced()) }
            Text(model.vpnStats).font(.caption.monospaced())
        }.navigationTitle("VPN")
        .onChange(of: model.vpnStatus) { _, status in
            if status == .connected { model.notice = "VPN connected; forwards stay active in the background"; dismiss() }
        }
    }
}
