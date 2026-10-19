[CCode (cheader_filename = "forwiroh.h")]
namespace Native {
    [CCode (cname = "forwiroh_version")]
    public unowned string version ();
    [CCode (cname = "forwiroh_crypto_info")]
    public unowned string crypto_info ();
    [CCode (cname = "forwiroh_qr_code")]
    public unowned string qr_code (string text);
    [CCode (cname = "forwiroh_open_desktop")]
    public void* open_desktop (string path, string name, out unowned string? error);
    [CCode (cname = "forwiroh_command")]
    public unowned string command (void* handle, string json);
    [CCode (cname = "forwiroh_next_event")]
    public unowned string? next_event (void* handle);
    [CCode (cname = "forwiroh_string_free")]
    public void string_free (void* text);
    [CCode (cname = "forwiroh_shutdown")]
    public void shutdown (void* handle);
    [CCode (cname = "forwiroh_free")]
    public void free (void* handle);
}
