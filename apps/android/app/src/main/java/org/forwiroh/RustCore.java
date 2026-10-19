package org.forwiroh;

import org.json.JSONObject;
import android.content.Context;
import java.nio.charset.StandardCharsets;

/** Owns the handle; shutdown wakes readers before we join calls and free it. */
final class RustCore implements AutoCloseable {
    static { System.loadLibrary("forwiroh_jni"); }
    private final long handle;
    private int calls;
    private boolean closing;
    private boolean closed;

    private static boolean initialized;
    private static synchronized void initialize(Context context) {
        if (!initialized) { initializeNative(context.getApplicationContext()); initialized = true; }
    }
    RustCore(Context context, String path, byte[] key) {
        initialize(context);
        handle = openNative(utf8(path), key);
        if (handle == 0) throw new IllegalStateException("Cannot start networking");
    }
    static byte[] utf8(String text) { return text.getBytes(StandardCharsets.UTF_8); }
    static String decode(byte[] bytes) { return new String(bytes, StandardCharsets.UTF_8); }
    static String version() { return decode(versionNative()); }
    private synchronized void enter() {
        if (closing) throw new IllegalStateException("Networking is stopped");
        calls++;
    }
    private synchronized void leave() { calls--; notifyAll(); }
    Object command(JSONObject command) throws Exception {
        enter();
        try {
            JSONObject reply = new JSONObject(decode(commandNative(handle, utf8(command.toString()))));
            if (reply.has("error")) throw new IllegalStateException(reply.getString("error"));
            return reply.get("data");
        } finally { leave(); }
    }
    JSONObject event() throws Exception {
        enter();
        try {
            byte[] bytes = eventNative(handle);
            return bytes == null ? null : new JSONObject(decode(bytes));
        } finally { leave(); }
    }
    boolean send(String session, byte[] packet, int length) {
        enter();
        try { return sendNative(handle, utf8(session), packet, length) == 0; }
        finally { leave(); }
    }
    byte[] receive(String session) {
        enter();
        try { return receiveNative(handle, utf8(session)); }
        finally { leave(); }
    }
    @Override public void close() {
        boolean interrupted = false;
        synchronized (this) {
            if (closing) {
                while (!closed) {
                    try { wait(); } catch (InterruptedException e) { interrupted = true; }
                }
                if (interrupted) Thread.currentThread().interrupt();
                return;
            }
            closing = true;
        }
        shutdownNative(handle);
        synchronized (this) {
            while (calls != 0) {
                try { wait(); } catch (InterruptedException e) { interrupted = true; }
            }
            freeNative(handle);
            closed = true;
            notifyAll();
        }
        if (interrupted) Thread.currentThread().interrupt();
    }
    private static native void initializeNative(Context context);
    private static native long openNative(byte[] path, byte[] key);
    private static native byte[] commandNative(long handle, byte[] json);
    private static native byte[] eventNative(long handle);
    private static native byte[] versionNative();
    private static native void shutdownNative(long handle);
    private static native void freeNative(long handle);
    private static native int sendNative(long handle, byte[] session, byte[] packet, int length);
    private static native byte[] receiveNative(long handle, byte[] session);
    static native int readTun(int fd, byte[] packet);
}
