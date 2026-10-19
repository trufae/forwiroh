namespace Forwiroh {
    private void clear_box (Gtk.Box box) {
        while (box.get_first_child () != null) box.remove (box.get_first_child ());
    }
    private Gtk.Label label (string text, bool selectable = false) {
        var widget = new Gtk.Label (text);
        widget.xalign = 0; widget.wrap = true; widget.selectable = selectable;
        return widget;
    }
    private Gtk.Button button (string title, owned GtkAction action) {
        var widget = new Gtk.Button.with_label (title);
        widget.clicked.connect (() => action ());
        return widget;
    }
    private delegate void GtkAction ();

    public class Window : Gtk.ApplicationWindow {
        private Core core = new Core ();
        private Gtk.Box body = new Gtk.Box (Gtk.Orientation.VERTICAL, 12);
        private Gtk.Box pending_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 8);
        private Gtk.ListBox peers_list = new Gtk.ListBox ();
        private Gtk.Stack stack = new Gtk.Stack ();
        private Gtk.Box detail = new Gtk.Box (Gtk.Orientation.VERTICAL, 16);
        private Gtk.Box tunnels_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 8);
        private Gtk.Label status = label ("Starting networking…");
        private Gtk.Label peer_status = label ("");
        private Gtk.Button pair_button;
        private TCPClientView tcp = new TCPClientView ();
        private Json.Array peers = new Json.Array ();
        private Json.Array tunnels = new Json.Array ();
        private Json.Array pending = new Json.Array ();
        private string selected = "";
        private string identity = "";
        private bool refreshing = false;
        private bool refresh_again = false;
        private bool quitting = false;
        private bool pairing = false;
        private Gtk.Window? pair_window;
        private Gtk.Box? pair_pending;
        private Gtk.DrawingArea? qr_area;
        private Gtk.Label? expiry_label;
        private Gtk.Button? copy_button;
        private Gtk.Button? generate_button;
        private string invitation_id = "";
        private string invitation_uri = "";
        private int64 invitation_expires = 0;
        private uint expiry_timer = 0;
        private Json.Array qr_pixels = new Json.Array ();
        private int qr_width = 0;

        public Window (Gtk.Application app) {
            Object (application: app, title: "Forwiroh", default_width: 1050, default_height: 730);
            var header = new Gtk.HeaderBar ();
            pair_button = button ("Pair device…", () => show_pairing ()); pair_button.sensitive = false;
            header.pack_end (pair_button); set_titlebar (header);
            body.margin_top = body.margin_bottom = body.margin_start = body.margin_end = 12;
            body.append (pending_box);
            var split = new Gtk.Paned (Gtk.Orientation.HORIZONTAL); split.vexpand = true; split.position = 235;
            var sidebar = new Gtk.Box (Gtk.Orientation.VERTICAL, 12);
            sidebar.append (label ("Paired devices"));
            var list_scroll = new Gtk.ScrolledWindow (); list_scroll.vexpand = true; list_scroll.child = peers_list;
            sidebar.append (list_scroll);
            sidebar.append (button ("TCP test client", () => stack.visible_child_name = "tcp"));
            sidebar.append (button ("Version & crypto", () => show_about ()));
            split.start_child = sidebar;
            detail.margin_top = detail.margin_bottom = detail.margin_start = detail.margin_end = 20;
            var details_scroll = new Gtk.ScrolledWindow (); details_scroll.child = detail;
            stack.add_named (details_scroll, "peer"); stack.add_named (tcp, "tcp");
            split.end_child = stack; body.append (split);
            status.add_css_class ("dim-label"); body.append (status); set_child (body);
            peers_list.row_selected.connect ((row) => {
                if (row == null) return;
                var value = peers.get_object_element ((uint) row.get_index ());
                selected = text (value, "id"); show_peer (value);
            });
            close_request.connect (() => {
                if (!quitting) {
                    quitting = true; sensitive = false; status.label = "Closing tunnels…";
                    tcp.close_connection ();
                    finish.begin ();
                }
                return true;
            });
            core.event_received.connect ((event) => {
                if (event.has_member ("error")) show_error (text (event, "error"));
                if (text (event, "event") == "paired") {
                    status.label = "Device paired";
                    if (pair_window != null && !pairing) pair_window.close ();
                }
                refresh.begin ();
            });
            rebuild_peers ();
            startup.begin ();
        }
        private async void startup () {
            try { yield core.open (); if (!quitting) { pair_button.sensitive = true; status.label = "Networking active"; yield refresh (); if (Environment.get_variable ("FORWIROH_UI_SMOKE") == "1") yield smoke_ui (); } }
            catch (Error e) {
                show_error (e.message);
                detail.append (label ("Could not start networking. State directory: " + core.path, true));
                detail.append (button ("Retry", () => startup.begin ()));
                if (Environment.get_variable ("FORWIROH_UI_SMOKE") == "1") { ((Application) application).test_result = 1; close (); }
            }
        }
        private async void smoke_ui () {
            stack.visible_child_name = "tcp";
            var about = show_about (); about.close ();
            show_pairing ();
            yield generate_invitation ();
            assert (qr_width > 20 && qr_pixels.get_length () == qr_width * qr_width);
            Timeout.add (300, () => { smoke_ui.callback (); return Source.REMOVE; });
            yield;
            if (pair_window != null) pair_window.close ();
            print ("GTK UI: startup, TCP chat, crypto, QR rendering and graceful close passed\n");
            close ();
        }
        private async void finish () {
            if (expiry_timer != 0) Source.remove (expiry_timer);
            yield core.close ();
            destroy ();
        }
        private void show_error (string message) { status.label = message; status.remove_css_class ("dim-label"); status.add_css_class ("error"); }
        private async bool run (Json.Node cmd, string message) {
            try {
                yield core.request (cmd);
                status.label = message; status.remove_css_class ("error");
                yield refresh (); return true;
            } catch (Error e) { if (!quitting) show_error (e.message); return false; }
        }
        private async void refresh () {
            if (!core.ready || quitting) return;
            if (refreshing) { refresh_again = true; return; }
            refreshing = true;
            do {
                refresh_again = false;
                try {
                    var snapshot = yield core.request (make_command ("status"));
                    var obj = snapshot.get_object ();
                    identity = text (obj, "id"); tunnels = array (obj, "tunnels");
                    var next = array (obj, "peers");
                    // Keep form entry and keyboard focus when only connection state changes.
                    bool changed = next.get_length () != peers.get_length ();
                    if (!changed) for (uint i = 0; i < next.get_length (); i++) {
                        if (text (next.get_object_element (i), "id") != text (peers.get_object_element (i), "id") ||
                            text (next.get_object_element (i), "name") != text (peers.get_object_element (i), "name")) changed = true;
                    }
                    peers = next;
                    if (changed) rebuild_peers ();
                    for (uint i = 0; i < peers.get_length (); i++) {
                        var peer = peers.get_object_element (i);
                        if (text (peer, "id") == selected) peer_status.label = boolean (peer, "connected") ? "Connected · " + text (peer, "path", "Iroh") : "Disconnected";
                    }
                    rebuild_tunnels ();
                    var requests = yield core.request (make_command ("pending"));
                    pending = requests.get_array ();
                    rebuild_pending (pending_box);
                    if (pair_pending != null) rebuild_pending (pair_pending);
                } catch (Error e) { if (!quitting) show_error (e.message); }
            } while (refresh_again && !quitting);
            refreshing = false;
        }
        private void rebuild_peers () {
            while (peers_list.get_first_child () != null) peers_list.remove (peers_list.get_first_child ());
            Gtk.ListBoxRow? chosen = null;
            for (uint i = 0; i < peers.get_length (); i++) {
                var peer = peers.get_object_element (i);
                var row = new Gtk.ListBoxRow ();
                var name = label (text (peer, "name")); name.margin_top = name.margin_bottom = 10;
                row.child = name; peers_list.append (row);
                if (text (peer, "id") == selected) chosen = row;
            }
            if (chosen != null) peers_list.select_row (chosen);
            else {
                selected = ""; clear_box (detail);
                detail.append (label ("Pair this computer with your iPhone or another computer to forward ports through encrypted Iroh connections."));
            }
        }
        private void rebuild_pending (Gtk.Box box) {
            clear_box (box);
            for (uint i = 0; i < pending.get_length (); i++) {
                var request = pending.get_object_element (i);
                string id = text (request, "id");
                box.append (label ("Pairing request from " + text (request, "name")));
                box.append (label (text (request, "peer"), true));
                box.append (label ("Check this identity on the requesting device. Approval allows TCP access to this computer."));
                var row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
                var approve_button = button ("Approve pairing", () => approve.begin (id, true, row));
                approve_button.add_css_class ("suggested-action");
                row.append (approve_button); row.append (button ("Reject", () => approve.begin (id, false, row))); box.append (row);
            }
        }
        private async void approve (string id, bool accept, Gtk.Box row) {
            if (!row.sensitive) return; row.sensitive = false;
            var cmd = make_command ("approve"); cmd.get_object ().set_string_member ("id", id); cmd.get_object ().set_boolean_member ("accept", accept);
            if (!(yield run (cmd, accept ? "Pairing approved" : "Pairing rejected"))) row.sensitive = true;
        }
        private void show_peer (Json.Object peer) {
            clear_box (detail); stack.visible_child_name = "peer";
            var heading = label (text (peer, "name")); heading.add_css_class ("title-1"); detail.append (heading);
            detail.append (label (selected, true));
            peer_status = label (boolean (peer, "connected") ? "Connected · " + text (peer, "path", "Iroh") : "Disconnected"); detail.append (peer_status);
            var row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            var connect = button ("Connect / refresh", () => connect_peer.begin (selected)); row.append (connect);
            row.append (button ("Disconnect", () => { var cmd = peer_command ("disconnect"); run.begin (cmd, "Disconnected"); })); detail.append (row);
            row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            row.append (button ("Forward a port…", () => tunnel_form ("forward")));
            row.append (button ("Reverse forward…", () => tunnel_form ("reverse")));
            row.append (button ("SOCKS5 proxy…", () => tunnel_form ("socks")));
            if (Environment.get_os_info ("ID") != null) row.append (button ("VPN…", () => tunnel_form ("vpn")));
            detail.append (row);
            detail.append (label ("Active tunnels")); tunnels_box = new Gtk.Box (Gtk.Orientation.VERTICAL, 8); detail.append (tunnels_box); rebuild_tunnels ();
            var policy = peer.get_object_member ("policy");
            var access = new Gtk.CheckButton.with_label ("Allow connections from this paired peer");
            access.active = !boolean (policy, "restricted");
            access.toggled.connect (() => {
                var cmd = peer_command ("policy"); var node = peer.get_member ("policy").copy ();
                node.get_object ().set_boolean_member ("restricted", !access.active); cmd.get_object ().set_member ("policy", node);
                run.begin (cmd, "Access settings saved");
            }); detail.append (access);
            var name = new Gtk.Entry (); name.text = text (peer, "name"); name.placeholder_text = "Device name";
            row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8); row.append (name);
            row.append (button ("Rename", () => { var cmd = peer_command ("rename"); cmd.get_object ().set_string_member ("name", name.text); run.begin (cmd, "Device renamed"); })); detail.append (row);
            var remove = button ("Remove and revoke…", () => confirm_revoke ()); remove.add_css_class ("destructive-action"); detail.append (remove);
        }
        private Json.Node peer_command (string op) { var cmd = make_command (op); cmd.get_object ().set_string_member ("peer", selected); return cmd; }
        private async void connect_peer (string id) {
            peer_status.label = "Connecting…";
            var cmd = make_command ("services"); cmd.get_object ().set_string_member ("peer", id);
            yield run (cmd, "Connected");
            yield refresh ();
        }
        private void rebuild_tunnels () {
            clear_box (tunnels_box);
            for (uint i = 0; i < tunnels.get_length (); i++) {
                var tunnel = tunnels.get_object_element (i);
                if (text (tunnel, "peer") != selected) continue;
                string id = text (tunnel, "id"); string kind = text (tunnel, "kind");
                var row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
                var description = label (kind + ": " + text (tunnel, "detail"), true); description.hexpand = true; row.append (description);
                row.append (button ("Stop", () => { var cmd = make_command (kind == "vpn" ? "stop_vpn" : "stop"); cmd.get_object ().set_string_member ("id", id); run.begin (cmd, "Tunnel stopped"); })); tunnels_box.append (row);
            }
            if (tunnels_box.get_first_child () == null) tunnels_box.append (label ("No active tunnels"));
        }
        private Gtk.Window make_modal (string title, Gtk.Box content, int width = 560) {
            var window = new Gtk.Window (); window.title = title; window.transient_for = this; window.modal = true; window.destroy_with_parent = true; window.default_width = width;
            content.margin_top = content.margin_bottom = content.margin_start = content.margin_end = 20;
            window.child = content; return window;
        }
        private void confirm_revoke () {
            var content = new Gtk.Box (Gtk.Orientation.VERTICAL, 16);
            content.append (label ("Revoke this device and close its tunnels?"));
            var window = make_modal ("Revoke device", content);
            var cmd = peer_command ("revoke");
            content.append (button ("Cancel", () => window.close ()));
            var revoke = button ("Revoke", () => { run.begin (cmd, "Device revoked"); window.close (); }); revoke.add_css_class ("destructive-action"); content.append (revoke); window.present ();
        }
        private void show_pairing () {
            if (pair_window != null) { pair_window.present (); return; }
            var content = new Gtk.Box (Gtk.Orientation.VERTICAL, 12);
            pair_pending = new Gtk.Box (Gtk.Orientation.VERTICAL, 8); content.append (pair_pending); rebuild_pending (pair_pending);
            qr_area = new Gtk.DrawingArea (); qr_area.content_width = 320; qr_area.content_height = 320; qr_area.halign = Gtk.Align.CENTER;
            qr_area.set_draw_func ((area, cr, width, height) => {
                cr.set_source_rgb (1, 1, 1); cr.paint ();
                if (qr_width == 0 || (invitation_expires != 0 && invitation_expires <= new DateTime.now_utc ().to_unix ())) return;
                double scale = Math.floor (double.min (width, height) / (qr_width + 8));
                double left = (width - scale * qr_width) / 2; double top = (height - scale * qr_width) / 2;
                cr.set_source_rgb (0, 0, 0);
                for (int y = 0; y < qr_width; y++) for (int x = 0; x < qr_width; x++) {
                    if (qr_pixels.get_boolean_element ((uint) (y * qr_width + x))) cr.rectangle (left + x * scale, top + y * scale, scale, scale);
                }
                cr.fill ();
            }); content.append (qr_area);
            expiry_label = label ("Scan the invitation in Forwiroh on your iPhone, then approve its identity here."); content.append (expiry_label);
            var row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            generate_button = button ("Generate QR invitation", () => generate_invitation.begin ()); row.append (generate_button);
            copy_button = button ("Copy invitation", () => { var clipboard = get_display ().get_clipboard (); clipboard.set_text (invitation_uri); }); copy_button.sensitive = false; row.append (copy_button); content.append (row);
            content.append (new Gtk.Separator (Gtk.Orientation.HORIZONTAL));
            content.append (label ("Or accept an invitation from another device"));
            var entry = new Gtk.Entry (); entry.placeholder_text = "Paste forwiroh://pair?data=…"; content.append (entry);
            var accept = new Gtk.Button.with_label ("Accept & pair"); content.append (accept);
            accept.clicked.connect (() => {
                if (pairing) return;
                var cmd = make_command ("pair"); cmd.get_object ().set_string_member ("uri", entry.text.strip ()); cmd.get_object ().set_string_member ("device", "Linux");
                pairing = true; accept.sensitive = false; accept.label = "Waiting for approval…";
                run.begin (cmd, "Device paired", (obj, res) => { bool success = run.end (res); pairing = false; accept.sensitive = true; accept.label = "Accept & pair"; if (success && pair_window != null) pair_window.close (); });
            });
            content.append (button ("Cancel outgoing pairing", () => run.begin (make_command ("cancel_pair"), "Pairing cancelled")));
            pair_window = make_modal ("Pair a device", content, 610);
            pair_window.close_request.connect (() => {
                if (pairing) return true;
                if (invitation_id != "") { var cmd = make_command ("cancel_invite"); cmd.get_object ().set_string_member ("id", invitation_id); run.begin (cmd, "Invitation closed"); }
                if (expiry_timer != 0) { Source.remove (expiry_timer); expiry_timer = 0; }
                invitation_id = invitation_uri = ""; qr_width = 0;
                pair_window = null; pair_pending = null; qr_area = null; expiry_label = null; copy_button = null; generate_button = null;
                return false;
            });
            pair_window.present ();
        }
        private async void generate_invitation () {
            if (generate_button == null) return;
            generate_button.sensitive = false;
            try {
                if (invitation_id != "") { var cancel = make_command ("cancel_invite"); cancel.get_object ().set_string_member ("id", invitation_id); yield core.request (cancel); }
                var response = yield core.request (make_command ("invite")); var obj = response.get_object ();
                string id = text (obj, "id");
                if (pair_window == null) { var cancel = make_command ("cancel_invite"); cancel.get_object ().set_string_member ("id", id); yield core.request (cancel); return; }
                invitation_id = id; invitation_uri = text (obj, "uri"); invitation_expires = obj.get_int_member ("expires");
                var matrix = unpack (take_string (Native.qr_code (invitation_uri))).get_object ();
                qr_width = (int) matrix.get_int_member ("width"); qr_pixels = array (matrix, "pixels"); qr_area.queue_draw (); copy_button.sensitive = true;
                if (expiry_timer != 0) Source.remove (expiry_timer);
                update_expiry ();
                expiry_timer = Timeout.add_seconds (1, () => { update_expiry (); return Source.CONTINUE; });
            } catch (Error e) { show_error (e.message); }
            if (generate_button != null) generate_button.sensitive = true;
        }
        private void update_expiry () {
            if (expiry_label == null) return;
            int64 remaining = invitation_expires - new DateTime.now_utc ().to_unix ();
            expiry_label.label = remaining > 0 ? "Scan with Forwiroh. Expires in %lld seconds.".printf (remaining) : "Invitation expired. Generate a new QR code.";
            if (copy_button != null) copy_button.sensitive = remaining > 0;
            if (qr_area != null) qr_area.queue_draw ();
        }
        private Gtk.Entry field (Gtk.Box content, string title, string value) {
            content.append (label (title)); var entry = new Gtk.Entry (); entry.text = value; content.append (entry); return entry;
        }
        private void tunnel_form (string kind) {
            var content = new Gtk.Box (Gtk.Orientation.VERTICAL, 12);
            string id = selected;
            content.append (label (kind == "reverse" ? "Listen on the paired device and forward to this computer." : kind == "vpn" ? "Linux VPN client: provision an owned TUN, assigned address and routes first (docs/vpn.md). The paired server's default VPN exit needs no TUN or root." : "Listen on this computer and connect through the paired device."));
            var port = field (content, kind == "vpn" ? "Existing TUN interface" : "Listening port", kind == "vpn" ? "forwiroh0" : kind == "socks" ? "1080" : "2222");
            var all = new Gtk.CheckButton.with_label ("Listen on all interfaces (0.0.0.0)"); if (kind == "forward") content.append (all);
            Gtk.Entry? host = null; Gtk.Entry? target_port = null;
            if (kind == "forward" || kind == "reverse") { host = field (content, kind == "reverse" ? "Destination IP on this computer" : "Destination host on the paired device", kind == "reverse" ? "127.0.0.1" : "localhost"); target_port = field (content, "Destination port", "22"); }
            Gtk.Entry? service = null;
            if (kind == "forward") service = field (content, "Saved service (only if the peer restricts access)", "ssh");
            Gtk.Entry? routes = null;
            if (kind == "vpn") routes = field (content, "Routes, separated by commas", "192.168.1.0/24");
            var window = make_modal (kind == "vpn" ? "Linux VPN" : kind == "socks" ? "SOCKS5 proxy" : "Port forwarding", content);
            var preview = label (""); content.append (preview);
            var form_error = label (""); form_error.add_css_class ("error"); content.append (form_error);
            if (host != null && target_port != null) {
                host.changed.connect (() => preview.label = (kind == "reverse" ? "Paired device" : "This computer") + " :" + port.text + " → " + (kind == "reverse" ? "This computer" : "Paired device") + " " + host.text + ":" + target_port.text);
                target_port.changed.connect (() => host.changed ()); port.changed.connect (() => host.changed ()); host.changed ();
            }
            var start = new Gtk.Button.with_label ("Start"); start.add_css_class ("suggested-action"); content.append (start);
            bool starting = false;
            window.close_request.connect (() => starting);
            start.clicked.connect (() => {
                if (starting) return;
                var cmd = make_command (kind); var obj = cmd.get_object (); obj.set_string_member ("peer", id);
                uint64 number = 0, target = 0;
                if (kind != "vpn" && (!uint64.try_parse (port.text, out number) || number == 0 || number > 65535)) { form_error.label = "Enter a listening port between 1 and 65535"; return; }
                if (host != null && target_port != null && (!uint64.try_parse (target_port.text, out target) || target == 0 || target > 65535 || host.text.strip () == "")) { form_error.label = "Enter a destination host and port between 1 and 65535"; return; }
                if (kind == "vpn") {
                    obj.set_string_member ("tun", port.text.strip ()); var networks = new Json.Array ();
                    foreach (var route in routes.text.split (",")) networks.add_string_element (route.strip ()); obj.set_array_member ("routes", networks);
                } else if (kind == "socks") obj.set_string_member ("listen", "127.0.0.1:%llu".printf (number));
                else {
                    string address = host.text.strip (); if (address.contains (":") && !address.has_prefix ("[")) address = "[" + address + "]";
                    if (kind == "reverse") { obj.set_int_member ("port", (int64) number); obj.set_string_member ("target", "%s:%llu".printf (address, target)); }
                    else { obj.set_string_member ("listen", "%s:%llu".printf (all.active ? "0.0.0.0" : "127.0.0.1", number)); obj.set_string_member ("service", "%s:%llu".printf (address, target)); }
                }
                starting = true; content.sensitive = false; start.label = "Starting…";
                start_tunnel.begin (cmd, service != null ? service.text : "", window, content, start, form_error, (source, res) => { start_tunnel.end (res); starting = false; });
            }); window.present ();
        }
        private async void start_tunnel (Json.Node cmd, string service, Gtk.Window window, Gtk.Box content, Gtk.Button start, Gtk.Label form_error) {
            try {
                if (text (cmd.get_object (), "op") == "forward") {
                    var query = make_command ("services"); query.get_object ().set_string_member ("peer", text (cmd.get_object (), "peer"));
                    var response = yield core.request (query);
                    if (boolean (response.get_object (), "restricted", true)) cmd.get_object ().set_string_member ("service", service);
                }
                yield core.request (cmd); status.label = "Tunnel started"; yield refresh (); window.destroy ();
            } catch (Error e) { show_error (e.message); form_error.label = e.message; content.sensitive = true; start.label = "Start"; }
        }
        private Gtk.Window show_about () {
            var content = new Gtk.Box (Gtk.Orientation.VERTICAL, 16);
            content.append (label ("Forwiroh " + Native.version ()));
            content.append (label ("Iroh · QUIC · TLS 1.3\nIdentity: Ed25519 · 256-bit public key"));
            try {
                var crypto = parse (take_string (Native.crypto_info ())).get_object ();
                content.append (label ("Provider: " + text (crypto, "provider")));
                foreach (var name in new string[] {"cipher_suites", "key_exchanges"}) {
                    var algorithms = array (crypto, name);
                    for (uint i = 0; i < algorithms.get_length (); i++) content.append (label (algorithms.get_string_element (i), true));
                }
            } catch (Error e) { content.append (label (e.message)); }
            content.append (label ("These are supported algorithms. Connections negotiate their cipher and key exchange. Relay servers cannot decrypt the tunnel contents."));
            content.append (label ("Device identity:\n" + identity, true));
            content.append (label ("Owner-only identity and pairing state:\n" + core.path, true));
            var window = make_modal ("Version & crypto", content); content.append (button ("Close", () => window.close ())); window.present (); return window;
        }
    }

    public class Application : Gtk.Application {
        public int test_result { get; set; default = 0; }
        public Application () { Object (application_id: "org.radare.forwiroh.gtk", flags: Environment.get_variable ("FORWIROH_UI_SMOKE") == "1" ? ApplicationFlags.NON_UNIQUE : ApplicationFlags.DEFAULT_FLAGS); }
        protected override void activate () {
            if (active_window != null) active_window.present ();
            else new Window (this).present ();
        }
    }
}
int main (string[] args) {
    if (args.length == 2 && args[1] == "--version") { print ("Forwiroh GTK %s\n", Native.version ()); return 0; }
    var app = new Forwiroh.Application ();
    int result = app.run (args);
    return app.test_result != 0 ? app.test_result : result;
}
