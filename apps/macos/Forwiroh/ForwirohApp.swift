import SwiftUI
import AppKit
import CoreImage
import Vision
import UniformTypeIdentifiers

@MainActor
final class AppDelegate: NSObject, NSApplicationDelegate {
    weak var model: Model?
    func applicationShouldTerminateAfterLastWindowClosed(_ sender: NSApplication) -> Bool { true }
    func applicationShouldTerminate(_ sender: NSApplication) -> NSApplication.TerminateReply {
        Task {
            await model?.stop()
            sender.reply(toApplicationShouldTerminate: true)
        }
        return .terminateLater
    }
}

@main
struct ForwirohApp: App {
    @NSApplicationDelegateAdaptor(AppDelegate.self) private var delegate
    @StateObject private var model = Model()
    var body: some Scene {
        WindowGroup {
            DesktopView().environmentObject(model)
                .frame(minWidth: 800, minHeight: 560)
                .task { delegate.model = model; await model.start() }
                .alert("Forwiroh", isPresented: Binding(get: { model.error != nil }, set: { if !$0 { model.error = nil } })) {
                    Button("OK") { model.error = nil }
                } message: { Text(model.error ?? "") }
        }
        .defaultSize(width: 1000, height: 720)
        .commands {
            CommandGroup(replacing: .newItem) {
                Button("Pair a device…") { model.showPairing = true }.keyboardShortcut("n").disabled(!model.ready)
            }
        }
    }
}

struct DesktopView: View {
    @EnvironmentObject private var model: Model
    @State private var selection: String?
    var body: some View {
        NavigationSplitView {
            List(selection: $selection) {
                Section("Paired devices") {
                    ForEach(model.peers) { peer in
                        NavigationLink(value: peer.id) {
                            VStack(alignment: .leading, spacing: 4) {
                                Label(peer.name, systemImage: peer.device == "iOS" ? "iphone" : "desktopcomputer")
                                Text(peer.connected ? "Connected · \(peer.path ?? "Iroh")" : "Disconnected")
                                    .font(.caption).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
                Section("Tools") {
                    NavigationLink("TCP test client", value: "tcp")
                    NavigationLink("Version & crypto", value: "about")
                }
            }.navigationTitle("Forwiroh").navigationSplitViewColumnWidth(min: 210, ideal: 240)
        } detail: {
            VStack(spacing: 0) {
                if !model.pending.isEmpty { PendingView().padding().background(.quaternary) }
                if selection == "tcp" { TCPConsoleView() }
                else if selection == "about" { AboutView() }
                else if let peer = model.peers.first(where: { $0.id == selection }) { PeerDetailView(peer: peer) }
                else {
                    ContentUnavailableView {
                        Label("Your devices, connected", systemImage: "network")
                    } description: {
                        Text("Pair this Mac with your iPhone or another computer to forward ports through encrypted Iroh connections.")
                    } actions: {
                        if model.starting { ProgressView("Starting networking…") }
                        else if model.ready { Button("Pair a device…") { model.showPairing = true } }
                        else { Button("Retry networking") { Task { await model.start() } } }
                    }
                }
                Divider()
                HStack {
                    Label(model.ready ? "Networking active" : model.starting ? "Starting…" : "Networking stopped", systemImage: model.ready ? "circle.fill" : "circle")
                        .foregroundStyle(model.ready ? .green : .secondary)
                    Spacer()
                    Text(model.notice).lineLimit(1)
                }.font(.caption).padding(10)
            }
        }
        .toolbar { Button("Pair device", systemImage: "qrcode") { model.showPairing = true }.disabled(!model.ready) }
        .sheet(isPresented: $model.showPairing, onDismiss: { Task { await model.cancelInvite() } }) { PairingView() }
    }
}

struct PendingView: View {
    @EnvironmentObject private var model: Model
    @State private var approving = false
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            ForEach(model.pending) { request in
                Text("Pairing request from \(request.name)").font(.headline)
                Text(request.peer).font(.caption.monospaced()).textSelection(.enabled)
                Text("Check this identity on the requesting device. Approving allows TCP access to this Mac.").font(.caption)
                HStack {
                    Button("Reject", role: .destructive) { answer(request.id, false) }
                    Button("Approve pairing") { answer(request.id, true) }.buttonStyle(.borderedProminent)
                    if approving { ProgressView().controlSize(.small) }
                }.disabled(approving)
            }
        }
    }
    private func answer(_ id: String, _ accept: Bool) {
        approving = true
        Task {
            defer { approving = false }
            _ = await model.run(["op": "approve", "id": id, "accept": accept], notice: accept ? "Pairing approved" : "Pairing rejected")
        }
    }
}

struct PairingView: View {
    @EnvironmentObject private var model: Model
    @Environment(\.dismiss) private var dismiss
    @State private var uri = ""
    @State private var showImporter = false
    @State private var importing = false
    var body: some View {
        VStack(spacing: 16) {
            HStack { Text("Pair a device").font(.title2.bold()); Spacer(); Button("Close") { dismiss() }.disabled(model.pairing) }
            ScrollView {
                VStack(spacing: 16) {
                    if !model.pending.isEmpty { PendingView() }
                    if let invite = model.invitation {
                        TimelineView(.periodic(from: .now, by: 1)) { timeline in
                            let expired = timeline.date >= invite.expires
                            if expired {
                                Text("Invitation expired").foregroundStyle(.secondary)
                                Button("Generate a new invitation") { Task { await model.invite() } }
                            } else {
                                QRView(uri: invite.uri).frame(width: 300, height: 300)
                                Text("Scan in Forwiroh on your iPhone, then approve its identity here.")
                                Text("Expires in \(Int(invite.expires.timeIntervalSince(timeline.date))) seconds").font(.caption).foregroundStyle(.secondary)
                                Button("Copy invitation") { NSPasteboard.general.clearContents(); NSPasteboard.general.setString(invite.uri, forType: .string) }
                            }
                        }
                    } else {
                        Button("Show QR invitation") { Task { await model.invite() } }.disabled(model.busy)
                        if model.busy { ProgressView() }
                    }
                    Divider()
                    Text("Or accept an invitation from another device").font(.headline)
                    TextField("Paste forwiroh://pair?data=…", text: $uri, axis: .vertical).lineLimit(2...4).textFieldStyle(.roundedBorder)
                    if let summary = invitationSummary(uri) { Text(summary).font(.caption.monospaced()).textSelection(.enabled) }
                    HStack {
                        Button(importing ? "Reading QR…" : "Read QR from image…") { showImporter = true }.disabled(importing || model.pairing)
                        Spacer()
                        if model.pairing { ProgressView().controlSize(.small); Button("Cancel") { Task { _ = await model.run(["op": "cancel_pair"], notice: "Pairing cancelled") } } }
                        Button(model.pairing ? "Waiting for approval…" : "Accept & pair") {
                            Task { if await model.pair(uri) { dismiss() } }
                        }.disabled(model.pairing || invitationSummary(uri) == nil)
                    }
                }.padding(2)
            }
        }.padding(24).frame(width: 610, height: 680)
            .interactiveDismissDisabled(model.pairing)
            .fileImporter(isPresented: $showImporter, allowedContentTypes: [.image]) { result in
                guard case .success(let url) = result else { return }
                importing = true
                Task {
                    defer { importing = false }
                    do {
                        uri = try await Task.detached {
                            let scoped = url.startAccessingSecurityScopedResource()
                            defer { if scoped { url.stopAccessingSecurityScopedResource() } }
                            let request = VNDetectBarcodesRequest()
                            request.symbologies = [.qr]
                            try VNImageRequestHandler(url: url).perform([request])
                            guard let value = request.results?.compactMap(\.payloadStringValue).first(where: { $0.hasPrefix("forwiroh://pair?data=") }) else {
                                throw CoreError(message: "No Forwiroh invitation found in this image")
                            }
                            return value
                        }.value
                    } catch { model.error = error.localizedDescription }
                }
            }
    }
}

func invitationSummary(_ uri: String) -> String? {
    let uri = uri.trimmingCharacters(in: .whitespacesAndNewlines)
    let prefix = "forwiroh://pair?data="
    guard uri.utf8.count <= 4096, uri.hasPrefix(prefix) else { return nil }
    var encoded = String(uri.dropFirst(prefix.count)).replacingOccurrences(of: "-", with: "+").replacingOccurrences(of: "_", with: "/")
    encoded += String(repeating: "=", count: (4 - encoded.count % 4) % 4)
    guard let data = Data(base64Encoded: encoded), let value = try? JSONSerialization.jsonObject(with: data) as? [String: Any],
          let name = value["name"] as? String, let addr = value["addr"] as? [String: Any], let id = addr["id"] as? String else { return nil }
    return "\(name)\n\(id)"
}

struct QRView: View {
    let uri: String
    private var image: NSImage? {
        guard let filter = CIFilter(name: "CIQRCodeGenerator") else { return nil }
        filter.setValue(Data(uri.utf8), forKey: "inputMessage")
        filter.setValue("M", forKey: "inputCorrectionLevel")
        guard let output = filter.outputImage, let cg = CIContext().createCGImage(output, from: output.extent) else { return nil }
        return NSImage(cgImage: cg, size: NSSize(width: cg.width, height: cg.height))
    }
    var body: some View {
        if let image { Image(nsImage: image).interpolation(.none).resizable().scaledToFit().padding(16).background(.white).accessibilityLabel("Forwiroh pairing QR code") }
        else { Text("Cannot render invitation QR") }
    }
}

enum TunnelKind: String, Identifiable { case forward, reverse, socks; var id: String { rawValue } }
struct PeerDetailView: View {
    @EnvironmentObject private var model: Model
    let peer: Peer
    @State private var connectionBusy = false
    @State private var restricted = false
    @State private var services: [String] = []
    @State private var form: TunnelKind?
    @State private var name = ""
    @State private var revoke = false
    @State private var accessBusy = false
    var body: some View {
        Form {
            Section("Connection") {
                LabeledContent("Device", value: peer.device)
                LabeledContent("Status", value: connectionBusy ? "Connecting…" : peer.connected ? "Connected · \(peer.path ?? "Iroh")" : "Disconnected")
                Text(peer.id).font(.caption.monospaced()).textSelection(.enabled)
                HStack {
                    Button(connectionBusy ? "Connecting…" : "Connect / refresh") { Task { await connect() } }
                    if connectionBusy { ProgressView().controlSize(.small) }
                    Button("Disconnect") { Task { _ = await model.run(["op": "disconnect", "peer": peer.id], notice: "Disconnected") } }
                }.disabled(connectionBusy || !model.ready)
            }
            Section("Port forwarding") {
                Text(restricted ? "This device permits its saved services and ports." : "Choose any destination host and port on \(peer.name). Paired requests are accepted and logged.").font(.callout)
                HStack {
                    Button("Forward a port…") { form = .forward }
                    Button("Reverse forward…") { form = .reverse }
                    Button("SOCKS5 proxy…") { form = .socks }
                }.disabled(connectionBusy || !model.ready)
            }
            Section("Active tunnels") {
                let active = model.tunnels.filter { $0.peer == peer.id }
                if active.isEmpty { Text("No active tunnels").foregroundStyle(.secondary) }
                ForEach(active) { tunnel in
                    HStack {
                        VStack(alignment: .leading) { Text(tunnel.kind.capitalized); Text(tunnel.detail).font(.caption.monospaced()) }
                        Spacer()
                        Button("Stop", role: .destructive) { Task { _ = await model.run(["op": "stop", "id": tunnel.id], notice: "Tunnel stopped") } }
                    }
                }
            }
            Section("Access to this Mac") {
                Toggle("Allow connections from this paired peer", isOn: Binding(get: { !(peer.policy["restricted"] as? Bool ?? false) }, set: { allow in
                    accessBusy = true
                    Task {
                        defer { accessBusy = false }
                        var policy = peer.policy; policy["restricted"] = !allow
                        _ = await model.run(["op": "policy", "peer": peer.id, "policy": policy], notice: "Access settings saved")
                    }
                })).disabled(accessBusy || !model.ready)
                Text("Turning this off applies this peer's configured service and port restrictions. Changing access closes its existing sessions.").font(.caption).foregroundStyle(.secondary)
            }
            Section("Device") {
                HStack { TextField("New name", text: $name); Button("Rename") { Task { _ = await model.run(["op": "rename", "peer": peer.id, "name": name], notice: "Device renamed") } }.disabled(name.isEmpty) }
                Button("Remove and revoke…", role: .destructive) { revoke = true }
            }.disabled(!model.ready)
        }.formStyle(.grouped).navigationTitle(peer.name)
            .task(id: peer.id) { name = peer.name; await connect() }
            .sheet(item: $form) { kind in TunnelForm(peer: peer, kind: kind, restricted: restricted, services: services) }
            .confirmationDialog("Revoke \(peer.name) and close its tunnels?", isPresented: $revoke) {
                Button("Revoke device", role: .destructive) { Task { _ = await model.run(["op": "revoke", "peer": peer.id], notice: "Device revoked") } }
            }
    }
    private func connect() async {
        guard !connectionBusy else { return }
        connectionBusy = true; defer { connectionBusy = false }
        do {
            let data = try await model.request(["op": "services", "peer": peer.id])
            restricted = data["restricted"] as? Bool ?? true
            services = data["services"] as? [String] ?? []
            await model.refresh()
        } catch { model.error = error.localizedDescription }
    }
}

struct TunnelForm: View {
    @EnvironmentObject private var model: Model
    @Environment(\.dismiss) private var dismiss
    let peer: Peer
    let kind: TunnelKind
    let restricted: Bool
    let services: [String]
    @State private var port = "2222"
    @State private var host = "localhost"
    @State private var targetPort = "22"
    @State private var service = "ssh"
    @State private var allInterfaces = false
    @State private var starting = false
    private var listener: String { kind == .reverse ? peer.name : "This Mac" }
    private var destinationDevice: String { kind == .reverse ? "This Mac" : peer.name }
    private var listenerHost: String { kind == .forward && allInterfaces ? "0.0.0.0" : "127.0.0.1" }
    var body: some View {
        VStack(alignment: .leading, spacing: 18) {
            Text(kind == .socks ? "SOCKS5 proxy" : kind == .reverse ? "Reverse forwarding" : "Port forwarding").font(.title2.bold())
            Form {
                if kind != .socks {
                    Text("\(listener) \(listenerHost):\(port) → \(destinationDevice) \(host):\(targetPort)").font(.callout.monospaced())
                }
                TextField("Listening port on \(listener)", text: $port)
                if kind == .forward { Toggle("Listen on all interfaces (0.0.0.0)", isOn: $allInterfaces) }
                if kind == .forward && restricted {
                    TextField("Saved service on \(peer.name)", text: $service)
                    if !services.isEmpty { Text("Available: " + services.joined(separator: ", ")).font(.caption) }
                } else if kind != .socks {
                    TextField(kind == .reverse ? "Destination IP on this Mac" : "Destination host on \(peer.name)", text: $host)
                    TextField("Destination port", text: $targetPort)
                }
                Text(kind == .socks ? "Configure your client to use this Mac's localhost:\(port). \(peer.name) resolves and connects to the destinations." : "Connections arriving on \(listener) are forwarded to \(destinationDevice).").font(.caption)
            }
            HStack {
                Button("Cancel") { dismiss() }.disabled(starting)
                Spacer()
                if starting { ProgressView().controlSize(.small) }
                Button(starting ? "Starting…" : "Start") { Task { await start() } }.buttonStyle(.borderedProminent).disabled(starting || !model.ready)
            }
        }.padding(24).frame(width: 570).disabled(starting).interactiveDismissDisabled(starting)
            .onAppear { if kind == .socks { port = "1080" } }
    }
    private func start() async {
        starting = true; defer { starting = false }
        guard let number = UInt16(port), number > 0 else { model.error = "Enter a listening port between 1 and 65535"; return }
        var command: [String: Any] = ["op": kind.rawValue, "peer": peer.id]
        if kind == .socks { command["listen"] = "127.0.0.1:\(number)" }
        else {
            guard let target = UInt16(targetPort), target > 0 else { model.error = "Enter a destination port between 1 and 65535"; return }
            let host = self.host.trimmingCharacters(in: .whitespacesAndNewlines)
            guard !host.isEmpty && !host.contains(where: \.isWhitespace) else { model.error = "Enter a destination host"; return }
            let normalized = kind == .reverse && host == "localhost" ? "127.0.0.1" : host
            let destination = "\(normalized.contains(":") && !normalized.hasPrefix("[") ? "[\(normalized)]" : normalized):\(target)"
            if kind == .reverse { command["port"] = Int(number); command["target"] = destination }
            else { command["listen"] = "\(listenerHost):\(number)"; command["service"] = restricted ? service : destination }
        }
        if await model.run(command, notice: "\(kind.rawValue.capitalized) started on \(listener):\(number)") { dismiss() }
    }
}

struct AboutView: View {
    @EnvironmentObject private var model: Model
    private let crypto = DesktopCore.cryptoInfo
    var body: some View {
        Form {
            Section("Forwiroh") {
                LabeledContent("App version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? DesktopCore.version)
                LabeledContent("Rust core", value: DesktopCore.version)
                LabeledContent("Transport", value: "Iroh / QUIC / TLS 1.3")
                Text(model.identity).font(.caption.monospaced()).textSelection(.enabled)
            }
            Section("Crypto capabilities") {
                LabeledContent("Identity", value: "Ed25519 · 256-bit public key")
                LabeledContent("Provider", value: crypto["provider"] as? String ?? "Unknown")
                ForEach(crypto["cipher_suites"] as? [String] ?? [], id: \.self) { Text($0).font(.caption.monospaced()) }
                ForEach(crypto["key_exchanges"] as? [String] ?? [], id: \.self) { Text($0).font(.caption.monospaced()) }
                Text("Supported algorithms; each connection negotiates its cipher and key exchange. Relay servers cannot decrypt the tunnel contents.").font(.caption)
            }
            Section("Local state") {
                Text(DesktopCore.statePath).font(.caption.monospaced()).textSelection(.enabled)
                Text("The identity and paired devices are stored with owner-only permissions. Keep this directory to preserve pairing.").font(.caption)
            }
        }.formStyle(.grouped).navigationTitle("Version & crypto")
    }
}

struct TCPConsoleView: View {
    @StateObject private var client = TCPClient()
    @State private var host = "127.0.0.1"
    @State private var port = "2222"
    @State private var input = ""
    @State private var crlf = true
    var body: some View {
        VStack(spacing: 12) {
            HStack {
                TextField("Host", text: $host).textFieldStyle(.roundedBorder)
                TextField("Port", text: $port).textFieldStyle(.roundedBorder).frame(width: 80)
                Button(client.connected || client.connecting ? "Disconnect" : "Connect") {
                    if client.connected || client.connecting { client.disconnect() } else { client.connect(host: host, port: port) }
                }
            }
            HStack { if client.connecting { ProgressView().controlSize(.small) }; Text(client.status).font(.caption); Spacer(); Button("Clear") { client.clear() } }
            ScrollViewReader { reader in
                ScrollView {
                    Text(client.output.isEmpty ? "Connect to see socket output here. SSH usually starts with an SSH-2.0 banner." : client.output)
                        .font(.system(.body, design: .monospaced)).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading)
                    Color.clear.frame(height: 1).id("end")
                }.background(.quaternary.opacity(0.3))
                    .onChange(of: client.output) { _, _ in reader.scrollTo("end", anchor: .bottom) }
            }
            Divider()
            HStack {
                TextField("Send text…", text: $input, axis: .vertical).lineLimit(1...5).textFieldStyle(.roundedBorder).onSubmit { send() }
                Button("Send", systemImage: "paperplane.fill") { send() }.disabled(!client.connected || client.sending || input.isEmpty)
            }
            HStack { Toggle("Append CRLF", isOn: $crlf); Spacer(); Text("Plain TCP · no terminal emulation").font(.caption).foregroundStyle(.secondary) }
        }.padding(20).navigationTitle("TCP test client").onDisappear { client.disconnect() }
    }
    private func send() {
        guard client.connected && !client.sending && !input.isEmpty else { return }
        client.send(input + (crlf ? "\r\n" : "")); input = ""
    }
}
