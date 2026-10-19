namespace Forwiroh {
    public class TCPClientView : Gtk.Box {
        private Gtk.Entry host = new Gtk.Entry ();
        private Gtk.Entry port = new Gtk.Entry ();
        private Gtk.Entry input = new Gtk.Entry ();
        private Gtk.CheckButton crlf = new Gtk.CheckButton.with_label ("Append CRLF");
        private Gtk.Label status = new Gtk.Label ("Disconnected");
        private Gtk.TextView console = new Gtk.TextView ();
        private Gtk.ScrolledWindow scroll = new Gtk.ScrolledWindow ();
        private Gtk.Button connect_button = new Gtk.Button.with_label ("Connect");
        private Gtk.Button send_button = new Gtk.Button.with_label ("Send");
        private SocketConnection? connection;
        private Cancellable? cancellation;
        private uint generation = 0;
        private bool connecting = false;
        private bool sending = false;

        public TCPClientView () {
            Object (orientation: Gtk.Orientation.VERTICAL, spacing: 12);
            margin_top = margin_bottom = margin_start = margin_end = 20;
            host.text = "127.0.0.1"; host.hexpand = true; host.placeholder_text = "Host";
            port.text = "2222"; port.width_chars = 6; port.placeholder_text = "Port";
            var row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            row.append (host); row.append (port); row.append (connect_button); append (row);
            connect_button.clicked.connect (() => {
                if (connecting || connection != null) close_connection ();
                else connect_socket.begin ();
            });
            var info = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            status.hexpand = true; status.xalign = 0; info.append (status);
            var clear = new Gtk.Button.with_label ("Clear");
            clear.clicked.connect (() => console.buffer.text = ""); info.append (clear); append (info);
            console.editable = false; console.cursor_visible = false; console.monospace = true;
            console.wrap_mode = Gtk.WrapMode.WORD_CHAR;
            scroll.child = console; scroll.vexpand = true; scroll.min_content_height = 250; append (scroll);
            input.placeholder_text = "Send text…"; input.hexpand = true;
            row = new Gtk.Box (Gtk.Orientation.HORIZONTAL, 8);
            row.append (input); row.append (send_button); append (row);
            crlf.active = true; append (crlf);
            var help = new Gtk.Label ("Plain TCP. SSH usually starts with an SSH-2.0 banner. No terminal emulation.");
            help.wrap = true; help.xalign = 0; help.add_css_class ("dim-label"); append (help);
            input.activate.connect (() => send_text.begin ());
            send_button.clicked.connect (() => send_text.begin ());
            update_buttons ();
        }
        private void update_buttons () {
            connect_button.label = connecting || connection != null ? "Disconnect" : "Connect";
            send_button.sensitive = connection != null && !sending;
            host.sensitive = port.sensitive = connection == null && !connecting;
        }
        public void close_connection (string reason = "Disconnected") {
            generation++;
            if (cancellation != null) cancellation.cancel ();
            var previous = connection; connection = null;
            if (previous != null) previous.close_async.begin (Priority.DEFAULT, null);
            connecting = false; sending = false; status.label = reason;
            update_buttons ();
        }
        private async void connect_socket () {
            close_connection ();
            uint64 number = 0;
            if (host.text.strip () == "" || !uint64.try_parse (port.text, out number) || number == 0 || number > 65535) {
                status.label = "Enter a host and a port between 1 and 65535"; return;
            }
            var attempt = generation;
            var cancel = new Cancellable (); cancellation = cancel;
            connecting = true; update_buttons ();
            console.buffer.text = ""; status.label = "Connecting…";
            var client = new SocketClient (); client.timeout = 15;
            try {
                var socket = yield client.connect_to_host_async (host.text.strip (), (uint16) number, cancel);
                if (attempt != generation) { socket.close_async.begin (Priority.DEFAULT, null); return; }
                connection = socket; connecting = false; status.label = "Connected"; update_buttons ();
                while (attempt == generation) {
                    var bytes = yield socket.input_stream.read_bytes_async (16384, Priority.DEFAULT, cancel);
                    if (attempt != generation) return;
                    if (bytes.get_size () == 0) { close_connection ("Remote closed the connection"); return; }
                    unowned uint8[] data = bytes.get_data ();
                    append_output (((string) data).make_valid (data.length));
                }
            } catch (Error e) { if (attempt == generation) close_connection ("Connection failed: " + e.message); }
        }
        private async void send_text () {
            if (connection == null || sending || input.text == "") return;
            var socket = connection;
            var attempt = generation;
            var payload = input.text + (crlf.active ? "\r\n" : "");
            sending = true; update_buttons ();
            try {
                size_t written;
                yield socket.output_stream.write_all_async (payload.data, Priority.DEFAULT, cancellation, out written);
                if (attempt != generation) return;
                append_output ("\n[sent] " + payload + (payload.has_suffix ("\n") ? "" : "\n"));
                input.text = ""; sending = false; update_buttons ();
            } catch (Error e) { if (attempt == generation) close_connection ("Send failed: " + e.message); }
        }
        private void append_output (string text) {
            Gtk.TextIter end;
            console.buffer.get_end_iter (out end); console.buffer.insert (ref end, text, -1);
            int extra = console.buffer.get_char_count () - 65536;
            if (extra > 0) {
                Gtk.TextIter start, cut;
                console.buffer.get_start_iter (out start); console.buffer.get_iter_at_offset (out cut, extra);
                console.buffer.delete (ref start, ref cut);
            }
            Idle.add (() => { var adjustment = scroll.vadjustment; adjustment.value = adjustment.upper - adjustment.page_size; return Source.REMOVE; });
        }
    }
}
