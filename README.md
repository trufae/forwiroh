<p align="center">
  <img src="docs/assets/forwiroh-logo.png" width="176" alt="Forwiroh — two linked endpoints" />
</p>

# Forwiroh

**Reach services on your own devices, wherever you are.**

Forwiroh connects an Android phone, iPhone, Linux machine or Mac to trusted peers
through encrypted [Iroh](https://iroh.computer/) connections. Run it on your home
computer, scan its pairing QR with your phone, approve the request, and open a
local port that reaches a service at home. You can pair several devices and run
several tunnels at once.

You do not need an email account, public IP address or router port forwarding.
Iroh tries a direct connection and can use an encrypted relay when necessary.
TCP forwarding and SOCKS5 work without root or a VPN interface.

This is an **early implementation**, under active validation. Platform builds and
automated tests do not replace physical-device testing. See the
[implementation checklist](docs/implementation.md), [Android guide](docs/android.md)
and [iOS guide](docs/ios.md) for verification details and limitations.

## What can I do with it?

| Feature | What it does | Example |
| --- | --- | --- |
| TCP forward | A port on this device connects to a destination on a peer. | Reach home SSH at `127.0.0.1:2222`, or a home web app at `127.0.0.1:8080`. |
| Reverse forward | A port on the peer connects back to a destination on this device. | Let the home machine reach your laptop's development server. |
| SOCKS5 proxy | SOCKS-aware applications send TCP connections through a chosen peer; the peer resolves destination names. | Use `curl` through your home network without changing system routes. |
| VPN | Selected routes, or full IPv4/IPv6 internet traffic on iOS, use a paired rootless exit peer. | Reach a private network or route phone traffic through a home peer. |
| Peer management | Pair, rename, disconnect, restrict or revoke trusted devices. | Keep separate home, laptop and phone peers. |

Pairing establishes trust; it does not automatically create tunnels or redirect
traffic. After pairing, choose one of the connection types above.

## Platforms

| Platform | Interface | Background networking |
| --- | --- | --- |
| Android 8.0+ | Native app in [`apps/android`](apps/android): QR scanner, peers, tunnels and VPN | A foreground service runs while its notification is active. |
| iPhone / iPad, iOS 17+ | Native SwiftUI app in [`apps/ios`](apps/ios): QR scanner, peers, tunnels, TCP diagnostics and VPN | Ordinary socket tunnels require the app in the foreground; VPN runs in a Packet Tunnel extension. |
| Linux | `forwiroh` CLI and daemon | Keep `serve` running, optionally as a user systemd service. Supports a pre-provisioned VPN TUN. |
| macOS | `forwiroh` CLI and daemon; native SwiftUI app in [`apps/macos`](apps/macos) | Keep `serve` running. CLI VPN is not implemented. |

The mobile apps and CLI share the same Rust networking and authorization core.
Android stores the identity encrypted with a Keystore key; iOS uses Keychain.
Saved peers persist. iOS restores saved forward, SOCKS and reverse profiles in
its VPN extension; other active tunnel configurations must be recreated after a
networking restart.

## Embed with SwiftPM

`import Forwiroh` gives your application an async Swift API for pairing, peer
management, permissions, TCP forwards, reverse forwards, SOCKS, events, and VPN
packet transport. The package includes prebuilt core libraries: consumers do not
need Rust, binding generation, a bridging header, or a separate daemon.
Swift 6.2 or newer is required.

| SDK platform | Included architectures | Minimum |
| --- | --- | --- |
| macOS | arm64 / x86_64 | macOS 14 |
| iOS | arm64 device; arm64 / x86_64 simulator | iOS 17 |
| Linux | arm64 / x86_64, glibc and musl | glibc 2.35 on x86_64; 2.39 on arm64, or musl |
| Android | arm64-v8a / x86_64 | API 28 |

Use a prepared checkout containing `Artifacts/`, or unpack
`Forwiroh-<version>-swiftpm.zip`. In the sibling project's
`pmai/MaiCore/Package.swift`, add the dependency and attach its product to the
target that will own the endpoint:

```swift
dependencies: [
    .package(path: "../../forwiroh"),
],
targets: [
    .target(name: "YourTarget", dependencies: [
        .product(name: "Forwiroh", package: "forwiroh"),
    ]),
]
```

For example, on macOS or Linux:

```swift
import Foundation
import Forwiroh

let state = URL.applicationSupportDirectory.appendingPathComponent("pmai/Forwiroh")
let node = try await Forwiroh.open(stateDirectory: state, name: "pmai")
let status = try await node.status()
print(status.id, status.peers)

// Once paired with "home", make its SSH service available locally.
let tunnel = try await node.forward(peer: "home", listen: "127.0.0.1:2222",
                                    service: "127.0.0.1:22")
// Use localhost:2222, then stop the tunnel when finished.
try await node.stopTunnel(id: tunnel)
await node.close()
```

Keep the endpoint alive for as long as the feature needs networking. Mobile
callers supply a persistent 32-byte identity from Keychain/Keystore. Android also
needs one-time JNI initialization and the certificate verifier's Java dependency.
System VPN setup and background services remain the host application's
responsibility. See the [SwiftPM integration guide](docs/swiftpm.md) for pairing,
events, mobile setup, lifecycle rules, and the
[standalone consumer example](examples/swiftpm).

Binaries are generated output and are not committed to this source tree. A fresh
clone needs a maintainer build before SwiftPM can consume it. The
[build and release instructions](docs/swiftpm.md#maintainer-builds-and-binary-releases)
and [SwiftPM workflow](.github/workflows/swiftpm.yml) produce Apple XCFrameworks,
Linux/Android static library bundles, and complete offline SwiftPM ZIPs in
`dist/`. No hosted binary release or SwiftPM distribution tag has been published;
use the prepared local checkout or full ZIP until then.

Validation includes six Swift tests on macOS and Linux, external SwiftPM
consumers, an unpacked standalone package, iOS device/simulator linking, and
Android native archive linking. Android Swift SDK builds are configured in CI;
Android device runtime testing is still outstanding.

## Quick start: phone → home computer

### 1. Build the home daemon

Install stable Rust (minimum 1.91), then:

```sh
git clone https://github.com/trufae/forwiroh.git
cd forwiroh
cargo install --path crates/forwiroh-cli --locked
forwiroh serve --name home
```

Leave that terminal running. Commands in other terminals talk to this daemon;
they do not launch another copy of the identity.

By default, state lives in `$XDG_STATE_HOME/forwiroh` or
`~/.local/state/forwiroh`. To use a different directory, pass `--state DIRECTORY`
to **every** command, including `serve`.

### 2. Install the phone app

- **Android:** open `apps/android` in Android Studio, or follow the
  [command-line build and APK installation instructions](docs/android.md#build).
  The APK supports arm64 and x86_64. Start networking in the app.
- **iOS:** open `apps/ios/Forwiroh.xcodeproj`, configure your signing team and
  entitlements, then run on your device. See [iOS signing/setup](docs/ios.md).

There is no App Store or Play Store distribution configured in this repository.
Android CI uploads a debug APK; iOS CI produces unsigned bundles for validation.

### 3. Pair once

In a second terminal on the home computer:

```sh
forwiroh pair
```

Scan the displayed QR in the phone app. Review and confirm the invitation, then
verify the requesting device identity in the home terminal and type `y` to approve.
Android can also accept a pasted invitation or display its own QR to another peer.

Invitations expire after five minutes and are single-use. If an invitation is
consumed or rejected, create a new one. Pairing can be cancelled with
`forwiroh cancel-pair` or the Android **Cancel outgoing pairing** control.

### 4. Open a service

Suppose the home computer has SSH on `127.0.0.1:22`. In the phone app, choose the
`home` peer and create a **TCP forward**:

| Setting | Value |
| --- | --- |
| Local listening port | `2222` |
| Remote destination / service | `127.0.0.1:22` |
| Listen on all interfaces | Off |

Now point an SSH client **on the phone** at `127.0.0.1`, port `2222`. Forwiroh carries
that TCP connection to SSH on the home computer. The destination's `127.0.0.1`
means the **home computer**; the SSH client's `127.0.0.1` means the **phone**.

For a home web app, use a local port such as `8080` and the remote destination
where that app listens. Open `http://127.0.0.1:8080` in a browser on the phone.
On iOS, switching away from Forwiroh suspends ordinary socket tunnels; use its
built-in TCP diagnostics for foreground checks or the separately configured VPN
for background network access. Android's notification keeps its service active.

Stop an individual tunnel in the app when finished. Android's **Stop all
networking** button or notification **Stop** action closes all socket tunnels and
VPN. On iOS, stop VPN separately in its VPN screen.

## Use a Linux or Mac client

On the client, start another daemon with its own state directory:

```sh
forwiroh serve --name laptop
```

In another client terminal, accept the home invitation on standard input so its
secret does not enter shell history. Paste the full invitation URI and press Return:

```sh
forwiroh pair --accept -
# Approve the request on the home computer, then:
forwiroh peers
forwiroh connect home
forwiroh forward home 2222:127.0.0.1:22
ssh -p 2222 user@127.0.0.1
```

Names are case-sensitive and must be unique to select a peer by name. An endpoint
ID identifies a peer unambiguously. To define a reusable destination name, run
this **on the machine hosting the service**:

```sh
forwiroh allow laptop ssh 127.0.0.1:22
```

### Reverse forward

Run this on the laptop to open a loopback listener on the home computer. Traffic
arriving at home port `8080` reaches the laptop's development server on port `3000`:

```sh
forwiroh reverse home 8080:127.0.0.1:3000
```

### SOCKS5

```sh
forwiroh socks home --listen 127.0.0.1:1080
curl --socks5-hostname 127.0.0.1:1080 https://example.com/
```

Only applications configured to use that proxy send traffic through it. SOCKS
supports TCP CONNECT, IPv4/IPv6 destinations and remote DNS. UDP ASSOCIATE is not
implemented.

### Inspect and stop

```sh
forwiroh status                 # peer connections and active tunnel IDs
forwiroh stop TUNNEL_ID
forwiroh disconnect home
forwiroh rename home home-server
forwiroh revoke home-server     # removes trust and terminates sessions
forwiroh --version
```

A disconnected TCP stream is never replayed. A new TCP connection may establish
a new Iroh session. For relay diagnostics, `forwiroh serve --relay-only` disables
direct UDP and requires relay transport.

## Trust and access restrictions

**Pair only devices you trust.** By default, a paired peer can open TCP and SOCKS
destinations and create reverse forwards on your device without another prompt.
Requests are logged by the receiving device. Unpaired devices cannot open tunnels.

Policies describe what a peer may access **on the device storing the policy**.
To restrict a peer, save a policy such as this and apply it with
`forwiroh policy PEER policy.json`:

```json
{
  "restricted": true,
  "services": {"ssh": "127.0.0.1:22"},
  "reverse_ports": [8080],
  "socks_networks": ["192.168.1.0/24"],
  "socks_ports": [80, 443]
}
```

Saving a policy closes existing sessions so removed grants take effect immediately.
With SOCKS restrictions enabled, both network and port must be granted; every
resolved address is checked before connecting to that numeric address. A saved
policy with no `restricted` field also uses the default open paired access.

Local forwarding listeners default to loopback. The mobile all-interfaces option
(`0.0.0.0`) lets other LAN devices use that port without local authentication;
enable it only when intended. SOCKS and reverse listeners remain loopback-only.
Use unprivileged listening ports, such as 2222, 8080 or 1080.

## Optional VPN setup

VPN needs more setup than TCP forwarding. The rootless exit on Linux or macOS
uses ordinary TCP/UDP sockets; legacy Linux clients still require a provisioned
TUN. Follow [the VPN setup guide](docs/vpn.md), then select the peer and routes
in the phone's VPN screen.

The current transport uses QUIC datagrams with MTU 1100. Android uses `VpnService`;
iOS uses `NEPacketTunnelProvider`, offering ports-only or full IPv4/IPv6 routing.
Android leaves DNS on the system network and excludes Forwiroh's own transport
sockets from VPN.

macOS system/CLI VPN, SOCKS UDP and automatic VPN reconnect are not implemented.
Chatmail integration is intentionally excluded; direct Iroh QR pairing is sufficient.

## Desktop apps

Native SwiftUI macOS and GTK4/Vala Linux apps live in `apps/macos` and `apps/gtk`.
They share the Rust core and provide peer pairing/approval with QR codes, TCP
forward/reverse/SOCKS controls, crypto/version information and a TCP test client.
See [desktop build and run instructions](docs/desktop.md).

## Start at login on Linux

The example service runs as your normal user; ordinary socket tunnels need no root:

```sh
mkdir -p ~/.local/state/forwiroh ~/.config/systemd/user
cp packaging/forwiroh.service ~/.config/systemd/user/
systemctl --user daemon-reload
systemctl --user enable --now forwiroh
```

Edit the service if Cargo installs binaries outside `~/.cargo/bin` or you use a
custom state directory. Configure user lingering separately for a headless server.

## Development and testing

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
python3 scripts/cli-smoke.py
# Disposable Linux VM with network namespace/TUN privileges:
sudo python3 scripts/vpn-kernel.py
```

Local core tests use real Iroh QUIC connections and TCP sockets with external
relays disabled. CI additionally checks public-relay transport, Linux kernel TUN
traffic, Linux/macOS CLI behavior, unsigned iOS builds and Android native builds,
lint and emulator integration tests. Android build/test commands are in
[docs/android.md](docs/android.md#verification). Physical camera, mobile VPN and
Wi-Fi/cellular transition testing remain separate validation tasks.

The workspace pins libc 0.2.188 while upstream Iroh dependencies migrate Apple
mobile declarations removed in libc 0.2.190. The shared logo and its platform
exports are documented in [docs/assets](docs/assets/README.md).
