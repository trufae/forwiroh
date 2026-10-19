# VPN and background forwarding

Use Forwiroh 0.2.0 or newer on **both** the iPhone and server. This release changes
VPN packet framing and the MTU. Existing identities and pairing survive an
upgrade; there is no need to scan another QR code.

## Rootless server

Run `forwiroh serve --name home` as your normal user, pair the phone and approve
it. That is all the server setup needed for the default VPN. Linux and macOS exits
translate IP packets into ordinary outgoing TCP and UDP sockets using smoltcp.
There is no server TUN, raw socket, root, CAP_NET_ADMIN, IP forwarding, firewall
change or NAT rule. The exit can reach the destinations its operating system and
network permit, including services on server localhost. IPv6 destinations need
working IPv6 on the server; IPv4 works independently.

Paired access is open by default, including VPN, and requests are logged. Unpaired
devices cannot start a VPN. A saved policy with `restricted: true` still requires
an explicit `vpn` grant. Revoke/disconnect or changing a policy closes live sessions
and their sockets. To restrict a rootless exit, grant permitted routes, including
the two virtual hosts if using the iPhone ports-only mode:

```json
{
  "restricted": true,
  "vpn": {
    "routes": ["10.250.0.1/32", "10.250.0.3/32", "192.168.1.0/24"],
    "local_ip": "10.250.0.1",
    "peer_ip": "10.250.0.2"
  }
}
```

Apply it with `forwiroh policy iPhone vpn-policy.json`. Policy replaces the whole
policy, so preserve existing service/SOCKS/reverse grants in that file if needed.
Omitting `tun` selects the rootless exit. An explicit VPN policy's route list is
an upper bound even when `restricted` is false. VPN grants are route grants;
the separate SOCKS port filters do not restrict VPN traffic.

## iPhone

Open the paired server, then VPN. Choose either:

- **Route all internet traffic:** IPv4 and IPv6 TCP/UDP, including DNS, use the
  paired exit. DNS defaults to `1.1.1.1`, reached through the exit; enter a different
  IPv4 resolver if desired.
- **Ports only:** disable that toggle. Only the virtual hosts and any extra routes
  use the VPN. Ordinary internet traffic keeps its existing connection. Optional
  DNS must be inside one of the selected routes.

The virtual hosts are:

| Address | Destination |
| --- | --- |
| `10.250.0.1:port` | `localhost:port` on the server, e.g. `10.250.0.1:22` for SSH |
| `10.250.0.3:port` | A listener/forward on this iPhone, e.g. `10.250.0.3:2222` |

For example, create the forward `iPhone localhost:2222 → server localhost:22`,
then enable VPN. Connect to `10.250.0.3:2222` from your SSH client. The existing
listener at `127.0.0.1:2222` also remains available. The all-interfaces checkbox
still permits connections to the phone's Wi-Fi address from another machine.

The app hands endpoint ownership and saved forwarding/SOCKS/reverse listeners
to the Packet Tunnel extension. This restarts existing streams once when VPN
starts; applications reconnect them. New connections and listeners then remain
in the extension when the containing app backgrounds. The app sends management
commands to the running extension, so forwards can be created/stopped there too.
Saved listeners are restored on the next start. Stop an individual listener to
remove it from the saved list.

The extension's own Iroh/native sockets bypass its VPN through Apple's NECP
provider-process policy. Full routing does not need fixed IP exceptions for a
peer or relay, so changing direct/relay paths does not trigger the former LAN
route-overlap rejection. Full mode enables `includeAllNetworks`; Apple's required
system/control-plane exceptions still apply. Stop VPN removes the system network
settings and DNS override. The app displays the extension's saved startup error
and the system's last disconnect error, and the extension logs startup/stops.

A signed physical iPhone is needed to validate actual suspension, sleep, memory,
Wi-Fi/cellular transitions and provisioning. Unsigned builds validate compilation.
Force quitting, system termination or loss of the peer can end a session. The app
currently reports a disconnection and requires reconnecting VPN; it does not
silently claim automatic reconnection. No GPS/background activity tricks are used.

## Packet transport and limits

VPN packets use encrypted Iroh QUIC datagrams with a random per-session tag,
framing version 2 and MTU 1280. Transport fragments are reassembled with a bound
of 128 pending packets, one MiB of allocated data and a two-second expiry. IP fragmentation for TCP/UDP is
reassembled separately with at most 16 pending packets, 128 ranges per packet and
a 30-second expiry. UDP replies are fragmented to the interface MTU; zero-byte
UDP datagrams are supported. Both IP families validate lengths and assigned
client source addresses; IPv4 also validates its header checksum. IPv4 options
and IPv6 extension headers other than fragmentation are unsupported.

There are at most eight sessions per endpoint, one session per paired peer, and
64 TCP plus 64 UDP flows per userspace exit. UDP mappings expire after two idle
minutes; another datagram opens a new mapping. Queues are bounded. Packet loss
or congestion can drop datagrams; inner TCP handles retransmission. This rootless
exit supports TCP/UDP applications, not arbitrary IP protocols such as ICMP ping,
multicast discovery or inbound internet connections to the phone.

## Legacy Linux TUN backend/client

A nonempty `vpn.tun` still selects the legacy kernel backend on Linux. This is
optional and requires administrative provisioning of an interface owned by the
daemon user, routes, forwarding/return routing and firewall/NAT as appropriate.
Each kernel-backend peer needs a unique TUN and client address. The daemon itself
does not configure these resources. Use **MTU 1280** on both ends.

The Linux CLI `forwiroh vpn home --route 192.168.1.0/24 --tun forwiroh0` is a
kernel **client** and likewise needs an administratively provisioned TUN, assigned
client address and routes. The rootless-server requirement does not remove the
OS privilege needed to install a VPN interface on a desktop client. CLI route
checks conservatively reject overlap with Iroh transport addresses; full desktop
routing needs explicit transport bypass and is not exposed as a working GUI VPN.
macOS CLI does not install a VPN interface. `forwiroh stop-vpn SESSION_ID` stops
the packet session; remove any manually installed routes/interface explicitly.

The isolated namespace test `scripts/vpn-kernel.py` verifies the legacy TUN path.
The ordinary Rust test suite verifies the rootless exit with real QUIC, TCP,
UDP/fragmentation, IPv6, virtual hosts, policy denial and revocation without TUN.
