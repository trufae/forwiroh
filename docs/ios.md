# Native iOS build and device validation

Requirements: Xcode with Swift 6 and an iOS SDK, Rust, and the Apple Rust targets:

```sh
rustup target add aarch64-apple-ios aarch64-apple-ios-sim
open apps/ios/Forwiroh.xcodeproj
```

The checked-in project has two targets: Forwiroh (SwiftUI) and PacketTunnel
(NetworkExtension), both in Swift 6 language mode. A build phase compiles the
shared Rust static library for the selected SDK/architecture. No Swift copy of
the networking protocol exists.
The project can be regenerated deterministically with
`python3 scripts/generate-xcode.py`; change the generator when changing targets
or build settings.

To build just the Rust library from the repository root, no environment variables
are needed:

```sh
apps/ios/build-rust.sh
```

This defaults to `iphoneos` / `arm64` (`aarch64-apple-ios`), discovers the SDK
with `xcrun`, and copies the library to `apps/ios/Frameworks/libforwiroh.a`.
Xcode's environment overrides these defaults when building the app or simulator.

In the forwarding form, **Listen on iPhone** means
`iPhone localhost:listening-port → peer destination-host:destination-port`.
For SSH on your Mac, choose iPhone, listening port `2222`, destination host
`localhost`, and destination port `22`. Choosing the peer as the listener reverses
the path: `peer localhost:listening-port → iPhone localhost:destination-port`.
Both ends and their ports are shown as you edit the form. Paired TCP, SOCKS and
reverse-forward requests are accepted and logged by default. Optional access
restrictions are configured on the device receiving the request.

To test the iPhone forward from your Mac, enable **Listen on all interfaces
(0.0.0.0)** under **Listen on iPhone**. With it off, the listener binds
`127.0.0.1`; with it on, it binds `0.0.0.0` and accepts connections to the iPhone's
Wi-Fi IPv4 address. For example, keep listening port `2222`, destination host
`localhost` and destination port `22`, then run this on your Mac while both
devices are on the same Wi-Fi:

```sh
ssh -p 2222 YOUR_MAC_USERNAME@IPHONE_WIFI_IP
```

Find the iPhone's IP address in Settings > Wi-Fi > the network's info button.
Connect to that address, not `0.0.0.0`. Keep Forwiroh open on the iPhone during
the test. The path is Mac > iPhone:2222 > paired Mac's localhost:22.

You can also test without leaving the iPhone app: open **TCP test client** under
**Tools**, or **Test a TCP connection** on the peer screen. Connect to
`127.0.0.1:2222` after starting a forward. An SSH destination should immediately
display its `SSH-2.0-…` banner. You can select any host/port, send text with an
optional CRLF newline, and read/copy received text in the console. This is a
chat-style screen: connection controls at the top, a scrolling console in the
middle, and the text composer pinned below it, above the keyboard. This is a
plain TCP diagnostic client; it does not implement SSH login or Telnet option
negotiation. It disconnects when leaving its screen or backgrounding the app.

Connection and tunnel setup show progress. Starting a forward or SOCKS listener
returns to the peer screen on success and displays a confirmation; errors keep
the form open. Ordinary forwards still stop when the iPhone app backgrounds.

The peer list's **About Forwiroh** section shows the installed app version/build
and the actual bundled Rust core version, even when the core is stopped or VPN
is active. The CLI displays its core version with `--version` or `version`;
`serve` prints it at startup and `status` reports the running daemon's version.

**About Forwiroh > Crypto information** displays Ed25519 identity key sizes,
Keychain storage, QUIC/TLS 1.3, and the cipher suites and key exchanges supported
by the compiled Rust TLS provider. Capabilities are read from the core even
when it is stopped; they are not a claim about a particular connection's
negotiated cipher.

For unsigned compilation (does not install a working VPN on a phone):

```sh
xcodebuild -project apps/ios/Forwiroh.xcodeproj -scheme Forwiroh \
  -configuration Release -sdk iphoneos -destination 'generic/platform=iOS' \
  ARCHS=arm64 CODE_SIGNING_ALLOWED=NO build
```

For a physical device, select your development team for BOTH targets. Register
unique app and extension bundle identifiers and replace the defaults as needed:

- app: `org.radare.forwiroh`
- extension: `org.radare.forwiroh.PacketTunnel`
- shared App Group build setting: `group.org.radare.forwiroh`
- shared Keychain suffix: `org.radare.forwiroh.shared`

Enable Network Extensions (packet-tunnel-provider), App Groups and Keychain
Sharing for the appropriate App IDs/profiles. The Keychain group includes the
team's `AppIdentifierPrefix`. Both targets must have access to the same groups.
The app derives the provider bundle ID by appending `.PacketTunnel` to its own
bundle ID. Do not put signing keys, certificates, profiles or a team credential
in source control. Provisioning and distribution eligibility must be checked in
your Apple developer account.

The app and extension cannot open the same state directory simultaneously. The
app shuts down Rust when entering the background and before starting VPN; the extension shuts down and joins its
packet/event readers before releasing the shared state. If starting the app
while VPN is connected, peer/status information and management commands use provider messages.
Saved forwarding/SOCKS/reverse listeners are restored in the extension, and the
VPN screen offers ports-only or full IPv4/IPv6 internet routing. Startup failures
are saved in the shared container and shown alongside the system disconnect error.
Neither process generates a replacement identity just because the Keychain is
locked or inaccessible: that is an error requiring user attention.

## Physical-device checklist (not yet verified)

1. Install a signed build with both extensions and correct shared entitlements.
2. Scan a QR displayed on a dark and a light terminal; test denied camera access.
3. Check both identities, approve/reject/expire/cancel pairing, restart the app.
4. Forward a service from a Linux host, including a large response and half-close.
5. Enable optional access restrictions and verify omitted services/SOCKS destinations are refused.
6. Upgrade the server to 0.1.0, run it as a normal user without TUN, start VPN.
   Test full IPv4/IPv6 internet TCP/UDP and DNS, then ports-only virtual hosts.
   Forward localhost:2222 to server localhost:22 and use 10.250.0.3:2222 from another app.
7. Lock the phone; test traffic, Wi-Fi/cellular changes, sleep and reconnection.
8. Stop VPN, verify route/DNS removal and that foreground networking can reopen.
   Background the app and reconnect from system Settings; verify state ownership.
9. Revoke the phone at the server during live TCP and VPN transfers.
10. Measure extension memory and battery use; validate relay fallback on real NATs.

Sources inspected during implementation:
- https://developer.apple.com/documentation/networkextension/nepackettunnelprovider
- https://developer.apple.com/documentation/networkextension/routing-your-vpn-network-traffic
- https://developer.apple.com/documentation/bundleresources/entitlements/com.apple.developer.networking.networkextension

An unsigned build is a compilation check, not proof of background VPN behavior,
provisioning correctness, App Store approval or successful physical-device use.
