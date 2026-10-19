# Native desktop apps

Both apps use the same Rust core and wire protocol as the CLI and iPhone. They
can pair with each other or the phone, approve/reject requests, rename/revoke
peers, forward/reverse TCP ports and run SOCKS5. They show Iroh path/state, active
tunnels, versions, identity and supported crypto algorithms. The TCP test client
puts its input below the console and sends optional CRLF lines.

These apps own their own persistent identity, independent of a CLI daemon. Close
one process before opening the same state directory elsewhere. Set
`FORWIROH_STATE=/path/to/state` to select another directory; the default desktop
state does not overwrite the CLI's state. A running desktop app can be an exit
for an iPhone VPN without root, using the same default paired access as `serve`.

## macOS / SwiftUI

Requires Xcode command-line tools with Swift 6, macOS 14 or newer, and stable Rust
(minimum 1.91). The app uses Swift 6 language mode. Build from the repository root:

```sh
./apps/macos/build.sh
open apps/macos/build/Forwiroh.app
```

The script builds the Rust static library and native SwiftUI executable, bundles
it and applies an ad-hoc development signature. Distribution/notarization is a
separate step. State defaults to `~/Library/Application Support/Forwiroh` with
owner-only files. Choose “Pair a device” to display/copy a QR invitation or accept
a pasted invitation; QR images can also be imported. Check identities before
approving. Pairing and network calls run off the UI thread; quit closes the core
and joins in-flight work before releasing its state lock.

The macOS app does not install a system VPN client/interface. Its forwards and
rootless VPN **exit** remain active while the desktop app runs in the background.

## Linux / GTK4 and Vala

Install GTK4 >= 4.8, json-glib >= 1.6, Vala, Meson, Ninja, a C compiler and Rust.
For Debian/Ubuntu, native development packages are:

```sh
sudo apt install build-essential pkg-config libgtk-4-dev libjson-glib-dev valac meson ninja-build
./apps/gtk/build.sh
./apps/gtk/build/forwiroh-gtk
```

State defaults to `$XDG_STATE_HOME/forwiroh-gtk` or `~/.local/state/forwiroh-gtk`.
Build output defaults to `apps/gtk/build`; `FORWIROH_GTK_BUILD_DIR` can override it.
Use “Pair a device” to generate a QR invitation or paste an invitation from
another device. Pending approval requests appear in the main window and pairing
window. Forwarding forms show progress and close after successful creation.

Install with `meson install -C apps/gtk/build` using the prefix selected at Meson
setup. The desktop entry and Rust shared library are included. The executable
has an install rpath to that prefix's library directory. `forwiroh-gtk --version`
reports the bundled core version.

GTK's Linux VPN **client** control attaches an already provisioned TUN and routes;
it does not silently configure a full system VPN. See [vpn.md](vpn.md). The
rootless server/exit needs neither this client configuration nor a TUN.

## Validation

`meson test -C apps/gtk/build --print-errorlogs` tests the asynchronous Vala/C/Rust
bridge with real pairing, QR matrices, TCP roundtrip, revocation, state locking,
identity persistence and shutdown during an active call. Headless GUI validation
uses:

```sh
FORWIROH_STATE=/tmp/forwiroh-ui-test FORWIROH_UI_SMOKE=1 G_DEBUG=fatal-criticals \
  xvfb-run -a dbus-run-session -- apps/gtk/build/forwiroh-gtk
```

Use a dedicated test state path. GUI smoke opens the TCP console, crypto dialog
and rendered QR invitation, then closes through the normal shutdown path.

A reproducible unprivileged Linux test image is available in
`scripts/desktop-linux.Dockerfile`; its test user is uid 1001 and no TUN or network
administration capabilities are needed for the ordinary Rust/desktop tests.
