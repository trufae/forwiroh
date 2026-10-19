namespace Forwiroh {
    public errordomain CoreError { FAILED }

    public string take_string (string text) {
        string copy = text;
        Native.string_free ((void*) text);
        return copy;
    }
    public Json.Node parse (string text) throws Error {
        var parser = new Json.Parser ();
        parser.load_from_data (text);
        return parser.get_root ().copy ();
    }
    public string encode (Json.Node node) {
        var generator = new Json.Generator ();
        generator.set_root (node);
        return generator.to_data (null);
    }
    public Json.Node make_command (string op) {
        var node = new Json.Node (Json.NodeType.OBJECT);
        var obj = new Json.Object ();
        obj.set_string_member ("op", op);
        node.set_object (obj);
        return node;
    }
    public string text (Json.Object obj, string member, string fallback = "") {
        if (!obj.has_member (member)) return fallback;
        var value = obj.get_member (member);
        return value.get_value_type () == typeof (string) ? value.get_string () : fallback;
    }
    public bool boolean (Json.Object obj, string member, bool fallback = false) {
        if (!obj.has_member (member)) return fallback;
        var value = obj.get_member (member);
        return value.get_value_type () == typeof (bool) ? value.get_boolean () : fallback;
    }
    public Json.Array array (Json.Object obj, string member) {
        if (obj.has_member (member) && obj.get_member (member).get_node_type () == Json.NodeType.ARRAY)
            return obj.get_array_member (member);
        return new Json.Array ();
    }
    public Json.Node unpack (string reply) throws Error {
        var root = parse (reply);
        if (root.get_node_type () != Json.NodeType.OBJECT) throw new CoreError.FAILED ("Invalid core response");
        var obj = root.get_object ();
        if (obj.has_member ("error")) throw new CoreError.FAILED (text (obj, "error", "Core error"));
        if (!obj.has_member ("data")) throw new CoreError.FAILED ("Core response has no data");
        return obj.get_member ("data").copy ();
    }

    // Native functions block. Every request and the event pump register a caller
    // before using the pointer. Shutdown wakes them, waits, then frees the handle.
    public class Core : Object {
        public signal void event_received (Json.Object event);
        public bool ready { get; private set; default = false; }
        public string path { get; private set; }
        private void* handle = null;
        private bool opening = false;
        private bool closing = false;
        private Mutex mutex;
        private Cond finished;
        private int callers = 0;

        public Core (string? state_path = null) {
            path = state_path ?? Environment.get_variable ("FORWIROH_STATE") ??
                Path.build_filename (Environment.get_user_state_dir (), "forwiroh-gtk");
        }
        public async void open (string name = "Linux") throws Error {
            if (ready || opening || closing) throw new CoreError.FAILED ("Networking is already starting or stopped");
            opening = true;
            void* opened = null;
            string? failure = null;
            new Thread<void*> ("forwiroh-open", () => {
                unowned string? error;
                opened = Native.open_desktop (path, name, out error);
                if (error != null) failure = take_string (error);
                Idle.add (() => { open.callback (); return Source.REMOVE; });
                return null;
            });
            yield;
            opening = false;
            handle = opened;
            if (handle == null) {
                if (failure != null) {
                    var reply = parse (failure);
                    throw new CoreError.FAILED (text (reply.get_object (), "error", "Cannot start networking"));
                }
                throw new CoreError.FAILED ("Cannot start networking");
            }
            if (closing) return;
            ready = true;
            add_caller ();
            var current = handle;
            new Thread<void*> ("forwiroh-events", () => {
                while (true) {
                    unowned string? raw = Native.next_event (current);
                    if (raw == null) break;
                    string json = take_string (raw);
                    Idle.add (() => {
                        if (!closing) {
                            try { event_received (parse (json).get_object ()); }
                            catch (Error e) { warning ("Invalid core event: %s", e.message); }
                        }
                        return Source.REMOVE;
                    });
                }
                remove_caller ();
                return null;
            });
        }
        private void add_caller () { mutex.lock (); callers++; mutex.unlock (); }
        private void remove_caller () { mutex.lock (); callers--; finished.broadcast (); mutex.unlock (); }
        public async Json.Node request (Json.Node command) throws Error {
            if (!ready || closing) throw new CoreError.FAILED ("Networking is stopped");
            var current = handle;
            var input = encode (command);
            string reply = "";
            add_caller ();
            new Thread<void*> ("forwiroh-command", () => {
                reply = take_string (Native.command (current, input));
                remove_caller ();
                Idle.add (() => { request.callback (); return Source.REMOVE; });
                return null;
            });
            yield;
            return unpack (reply);
        }
        public async void close () {
            if (closing) return;
            closing = true; ready = false;
            while (opening) {
                Timeout.add (20, () => { close.callback (); return Source.REMOVE; });
                yield;
            }
            if (handle == null) return;
            var current = handle;
            new Thread<void*> ("forwiroh-close", () => {
                Native.shutdown (current);
                mutex.lock ();
                while (callers > 0) finished.wait (mutex);
                mutex.unlock ();
                Native.free (current);
                Idle.add (() => { close.callback (); return Source.REMOVE; });
                return null;
            });
            yield;
            handle = null;
        }
    }
}
