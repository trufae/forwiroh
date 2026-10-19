# Android

The Java Android app in `apps/android` shares the Rust C ABI with iOS. There is
no second networking implementation. Minimum Android version is 8.0 (API 26);
the APK includes arm64-v8a and x86_64, targets API 35 and uses 16 KB native-library
alignment. ARMv7 is not packaged.

## Build

Install JDK 17, stable Rust (the workspace requires at least 1.91), and the Android
SDK. Set `ANDROID_HOME` to the SDK directory, or set `sdk.dir` in the untracked
`apps/android/local.properties`. Install these SDK packages and Rust targets:

```sh
sdkmanager 'platforms;android-35' 'build-tools;35.0.0' 'ndk;28.0.13004108' 'cmake;3.22.1'
rustup target add aarch64-linux-android x86_64-linux-android
cd apps/android
./gradlew assembleDebug assembleDebugAndroidTest lintDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Gradle invokes CMake, which builds the locked Rust library for each ABI and links
the JNI shared library. The Android static archive uses release optimization
with root-crate LTO disabled; the NDK performs the final shared-library link. Rust must be on Android Studio's inherited PATH. The
wrapper pins Gradle 8.11.1 with its distribution SHA-256. Dependencies come from
Google Maven, Maven Central and the upstream rustls verifier Maven archive.
The verifier JVM component version is read from Cargo.lock. JNI installs a
process-lifetime Application context for Android system DNS and TLS verification
before the first Rust endpoint is opened. Release signing credentials are deliberately not
part of this repository; configure your own signing for distribution. CI uploads
an installable debug APK and test/lint reports.

## Pair and connect

1. Run `forwiroh serve --name home` on Linux/macOS, then `forwiroh pair` in another
   terminal using the same state directory.
2. In Android, scan that QR or paste its invitation URI. Review and confirm it.
3. Verify the requesting device identity in the terminal and approve it.
4. Choose **Manage** on a peer to start TCP forwarding, reverse forwarding or
   SOCKS5. The remote destination can be a service name or numeric IP:port.
   Reverse forwarding requires a numeric local IP:port. All ports must be 1–65535.
5. Use the active tunnel's **Stop** control, or **Stop all networking** to close
   everything. Trust and names persist; active tunnels do not survive a restart.

Android can also display an invitation QR and approve/reject incoming requests.
Invitations expire after five minutes. After scanning, choose **Review requests**
to approve the requesting identity; **Cancel invitation** revokes the QR and any
associated pending request. Trust grants TCP access by default, just like the CLI. **Edit inbound
policy** accepts the shared policy JSON and validates it in Rust; its rules govern
access to this Android device. Saving closes existing peer sessions. **Revoke
trust** removes the peer and terminates sessions.

SOCKS5 listens on loopback; configure a SOCKS-aware application explicitly. It does
not redirect Android traffic. TCP forwards default to loopback; the explicit
all-interfaces checkbox exposes a listening port on the LAN without local client
authentication. Reverse listeners remain loopback-only on the remote peer.

## Background ownership and identity

One foreground service owns the Rust endpoint for socket tunnels and VPN. The
notification opens the app and offers **Stop**, including while the UI is bound.
A denied notification permission does not grant any extra background exemption;
Android still lists the foreground service in its active-app controls. Service
restart is non-sticky: after process death, reopen the app and recreate tunnels.
Android network callbacks notify Iroh when the default network changes. Battery
restrictions, force stop, and network transitions may still interrupt sessions.

The 32-byte Iroh identity is encrypted with an AES-GCM key held in Android
Keystore. Only authenticated ciphertext and its nonce enter the app's no-backup
storage; the Rust store receives the decrypted key in memory. Backup is disabled.
A lost wrapping key or corrupt identity fails visibly instead of silently changing
identity. Clearing app data intentionally loses trust and identity.

JNI uses ordinary UTF-8 byte arrays, including for emoji names. Closing a core
rejects new calls, wakes native readers, joins in-flight calls and only then frees
the handle. Blocking commands and packet reads never run on the UI thread.

## VPN

First configure the Linux peer's VPN policy, pre-provisioned TUN, forwarding and
return routes as described in [vpn.md](vpn.md). Android is a VPN client, not a
Linux-style TUN server. Select a paired peer, enter comma-separated private IPv4
CIDRs and accept Android's system VPN consent. Rust validates and negotiates the
routes; Android installs only those returned by the core with MTU 1100.

The Forwiroh package is excluded from VPN to keep Iroh's UDP and relay connections
outside the tunnel. Other applications use the specified split routes. DNS stays
on the system network. There is no full tunnel, custom DNS, IPv6 VPN, always-on
VPN, automatic reconnect, or SOCKS UDP support. Stopping VPN, revoking system VPN
consent or losing the peer closes the TUN and removes its routes. Socket tunnels
can continue independently. Choose **Stop all networking** to close both.

## Verification

```sh
cd apps/android
./gradlew connectedDebugAndroidTest
```

Run on an arm64 device or x86_64 emulator with networking enabled. Instrumented
tests exercise Keystore persistence, close/reopen identity, concurrent JNI
shutdown, pairing and approval between two real Rust endpoints, emoji round trips,
TCP forwarding with half-close, tunnel cleanup, trust revocation, and activity
recreation. CI runs these on an API 35 x86_64 emulator after building both ABIs.
The shared Rust workspace also tests reverse forwarding, SOCKS and VPN datagrams.

Physical-device camera scanning, VPN traffic to a provisioned Linux TUN, Android
system VPN revocation, and Wi-Fi/cellular transitions need device validation.
An APK build or emulator JNI test alone does not establish those results.
