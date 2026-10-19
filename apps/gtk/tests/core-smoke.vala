using Forwiroh;

async void smoke_pause () {
    Timeout.add (20, () => { smoke_pause.callback (); return Source.REMOVE; });
    yield;
}
async void smoke_echo (SocketConnection connection) {
    try {
        while (true) {
            var data = yield connection.input_stream.read_bytes_async (16384);
            if (data.get_size () == 0) break;
            size_t written;
            yield connection.output_stream.write_all_async (data.get_data (), Priority.DEFAULT, null, out written);
        }
        yield connection.close_async ();
    } catch (Error e) { warning ("Echo: %s", e.message); }
}
async Json.Node wait_pending (Core core) throws Error {
    for (int i = 0; i < 750; i++) {
        var result = yield core.request (make_command ("pending"));
        if (result.get_array ().get_length () > 0) return result.get_array ().get_element (0).copy ();
        yield smoke_pause ();
    }
    throw new CoreError.FAILED ("Pairing request did not arrive");
}
void remove_state (string path) throws Error {
    var dir = Dir.open (path);
    string? name;
    while ((name = dir.read_name ()) != null) FileUtils.remove (Path.build_filename (path, name));
    DirUtils.remove (path);
}
async void smoke () throws Error {
    string home_path = DirUtils.make_tmp ("forwiroh-gtk-home-XXXXXX");
    string phone_path = DirUtils.make_tmp ("forwiroh-gtk-phone-XXXXXX");
    var home = new Core (home_path); var phone = new Core (phone_path);
    uint heartbeat = 0;
    uint timer = Timeout.add (10, () => { heartbeat++; return Source.CONTINUE; });
    Error? failure = null;
    try {
        yield home.open ("home"); yield phone.open ("phone");
        var initial = yield home.request (make_command ("status"));
        string home_id = text (initial.get_object (), "id");
        var phone_status = yield phone.request (make_command ("status"));
        string phone_id = text (phone_status.get_object (), "id");
        var duplicate = new Core (home_path);
        try { yield duplicate.open (); throw new CoreError.FAILED ("State lock was not enforced"); }
        catch (Error e) { assert (e.message.contains ("already in use")); }
        var invitation = yield home.request (make_command ("invite"));
        string uri = text (invitation.get_object (), "uri");
        var qr = unpack (take_string (Native.qr_code (uri))).get_object ();
        int width = (int) qr.get_int_member ("width");
        assert (width > 20 && array (qr, "pixels").get_length () == width * width);
        var cmd = make_command ("pair"); cmd.get_object ().set_string_member ("uri", uri); cmd.get_object ().set_string_member ("device", "GTK test");
        bool paired = false; string? pair_error = null;
        phone.request.begin (cmd, (obj, res) => {
            try { phone.request.end (res); } catch (Error e) { pair_error = e.message; }
            paired = true;
        });
        var request = yield wait_pending (home);
        cmd = make_command ("approve"); cmd.get_object ().set_string_member ("id", text (request.get_object (), "id")); cmd.get_object ().set_boolean_member ("accept", true);
        yield home.request (cmd);
        while (!paired) yield smoke_pause ();
        assert (pair_error == null);
        var service = new SocketService ();
        uint16 port = service.add_any_inet_port (null);
        service.incoming.connect ((connection, obj) => { smoke_echo.begin (connection); return true; }); service.start ();
        cmd = make_command ("forward"); cmd.get_object ().set_string_member ("peer", home_id); cmd.get_object ().set_string_member ("listen", "127.0.0.1:0"); cmd.get_object ().set_string_member ("service", "127.0.0.1:%u".printf (port));
        var tunnel = yield phone.request (cmd);
        string tunnel_id = text (tunnel.get_object (), "id");
        var snapshot = yield phone.request (make_command ("status"));
        var tunnels = array (snapshot.get_object (), "tunnels");
        string listen = text (tunnels.get_object_element (0), "detail").split (" → ")[0];
        var client = new SocketClient (); client.timeout = 10;
        var socket = yield client.connect_to_host_async (listen, 0);
        string payload = "GTK async bridge roundtrip\n";
        size_t written;
        yield socket.output_stream.write_all_async (payload.data, Priority.DEFAULT, null, out written);
        var received = new DataInputStream (socket.input_stream);
        var line = yield received.read_line_async ();
        assert (line == "GTK async bridge roundtrip");
        yield socket.close_async ();
        cmd = make_command ("stop"); cmd.get_object ().set_string_member ("id", tunnel_id); yield phone.request (cmd);
        service.stop (); service.close ();
        cmd = make_command ("revoke"); cmd.get_object ().set_string_member ("peer", phone_id); yield home.request (cmd);
        cmd = make_command ("revoke"); cmd.get_object ().set_string_member ("peer", home_id); yield phone.request (cmd);
        invitation = yield home.request (make_command ("invite"));
        cmd = make_command ("pair"); cmd.get_object ().set_string_member ("uri", text (invitation.get_object (), "uri")); cmd.get_object ().set_string_member ("device", "GTK test");
        paired = false;
        phone.request.begin (cmd, (obj, res) => { try { phone.request.end (res); } catch (Error e) { pair_error = e.message; } paired = true; });
        yield wait_pending (home);
        yield phone.close (); while (!paired) yield smoke_pause ();
        assert (pair_error != null);
        yield home.close ();
        home = new Core (home_path); yield home.open ("home");
        snapshot = yield home.request (make_command ("status")); assert (text (snapshot.get_object (), "id") == home_id);
        assert (heartbeat > 10);
        print ("GTK core: async pairing, QR, TCP roundtrip, revocation, active-call shutdown and identity persistence passed\n");
    } catch (Error e) { failure = e; }
    yield phone.close (); yield home.close ();
    Source.remove (timer); remove_state (home_path); remove_state (phone_path);
    if (failure != null) throw failure;
}
int main () {
    var loop = new MainLoop (); int result = 0;
    smoke.begin ((obj, res) => { try { smoke.end (res); } catch (Error e) { stderr.printf ("%s\n", e.message); result = 1; } loop.quit (); });
    loop.run (); return result;
}
