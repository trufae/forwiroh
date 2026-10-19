import SwiftUI
import Network

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
