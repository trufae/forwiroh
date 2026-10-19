package org.forwiroh;

import android.content.Context;
import androidx.test.platform.app.InstrumentationRegistry;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import org.json.*;
import org.junit.Test;
import org.junit.runner.RunWith;
import java.io.*;
import java.net.*;
import java.nio.file.Files;
import java.util.*;
import java.util.concurrent.*;
import static org.junit.Assert.*;
import static org.forwiroh.NetworkService.json;

@RunWith(AndroidJUnit4.class)
public class CoreTest {
    private final Context context = InstrumentationRegistry.getInstrumentation().getTargetContext();
    private RustCore open(String name, int seed) throws Exception {
        File directory = Files.createTempDirectory(context.getCacheDir().toPath(), name).toFile();
        byte[] key = new byte[32]; Arrays.fill(key, (byte)seed);
        return new RustCore(context, directory.getPath(), key);
    }
    @Test public void identityPersistsEncryptedAndCoreReopens() throws Exception {
        assertNotNull(Class.forName("org.rustls.platformverifier.CertificateVerifier"));
        byte[] first = Identity.load(context), second = Identity.load(context);
        assertEquals(32, first.length); assertArrayEquals(first, second);
        byte[] stored = Files.readAllBytes(new File(context.getNoBackupFilesDir(), "identity.enc").toPath());
        assertEquals(60, stored.length);
        assertFalse(Arrays.equals(first, Arrays.copyOfRange(stored, 12, 44)));
        File path = Files.createTempDirectory(context.getCacheDir().toPath(), "persistent").toFile();
        String id;
        try (RustCore core = new RustCore(context, path.getPath(), first)) {
            id = ((JSONObject)core.command(json("op", "status"))).getString("id");
            assertFalse(RustCore.version().isEmpty());
            assertEquals(JSONObject.NULL, core.command(json("op", "network_change")));
        }
        try (RustCore core = new RustCore(context, path.getPath(), second)) {
            assertEquals(id, ((JSONObject)core.command(json("op", "status"))).getString("id"));
        }
        Arrays.fill(first, (byte)0); Arrays.fill(second, (byte)0);
    }
    @Test public void shutdownWakesBlockedReadersAndRejectsNewCalls() throws Exception {
        RustCore core = open("shutdown", 31);
        ExecutorService workers = Executors.newFixedThreadPool(2);
        try {
            Future<?> event = workers.submit(() -> {
                try { core.event(); } catch (Exception e) {
                    // close may win before the reader enters; either result is safe.
                    assertTrue(e.getMessage().contains("stopped"));
                }
            });
            Future<?> close = workers.submit(core::close);
            core.close(); close.get(10, TimeUnit.SECONDS); event.get(10, TimeUnit.SECONDS);
            try { core.command(json("op", "status")); fail("closed handle accepted a command"); }
            catch (IllegalStateException expected) { assertTrue(expected.getMessage().contains("stopped")); }
        } finally { core.close(); workers.shutdownNow(); }
    }
    @Test public void pairingTcpForwardAndRevocationUseRealRustTransport() throws Exception {
        ExecutorService workers = Executors.newFixedThreadPool(2);
        try (RustCore server = open("server", 41); RustCore client = open("client", 42);
             ServerSocket echo = new ServerSocket(0, 1, InetAddress.getByName("127.0.0.1"))) {
            echo.setSoTimeout(20000);
            String uri = ((JSONObject)server.command(json("op", "invite"))).getString("uri");
            Future<Object> paired = workers.submit(() -> client.command(json("op", "pair", "uri", uri, "device", "Android")));
            JSONArray pending = new JSONArray();
            long deadline = System.nanoTime() + TimeUnit.SECONDS.toNanos(20);
            while (pending.length() == 0 && System.nanoTime() < deadline) {
                pending = (JSONArray)server.command(json("op", "pending")); Thread.sleep(50);
            }
            assertEquals(1, pending.length());
            server.command(json("op", "approve", "id", pending.getJSONObject(0).getString("id"), "accept", true));
            paired.get(20, TimeUnit.SECONDS);
            String serverId = ((JSONObject)server.command(json("op", "status"))).getString("id");
            String clientId = ((JSONObject)client.command(json("op", "status"))).getString("id");
            client.command(json("op", "rename", "peer", serverId, "name", "Home 🐱"));
            assertEquals("Home 🐱", ((JSONObject)client.command(json("op", "status"))).getJSONArray("peers").getJSONObject(0).getString("name"));
            int port;
            try (ServerSocket unused = new ServerSocket(0)) { port = unused.getLocalPort(); }
            JSONObject tunnel = (JSONObject)client.command(json("op", "forward", "peer", serverId,
                    "listen", "127.0.0.1:" + port, "service", "127.0.0.1:" + echo.getLocalPort()));
            byte[] message = RustCore.utf8("Android JNI → Iroh → TCP 🐱");
            Future<?> echoed = workers.submit(() -> {
                try (Socket socket = echo.accept()) {
                    socket.setSoTimeout(10000);
                    byte[] received = new byte[message.length];
                    new DataInputStream(socket.getInputStream()).readFully(received);
                    assertArrayEquals(message, received);
                    socket.getOutputStream().write(received);
                } catch (IOException e) { throw new UncheckedIOException(e); }
            });
            try (Socket socket = new Socket("127.0.0.1", port)) {
                socket.setSoTimeout(10000); socket.getOutputStream().write(message); socket.shutdownOutput();
                byte[] received = new byte[message.length];
                new DataInputStream(socket.getInputStream()).readFully(received);
                assertArrayEquals(message, received); assertEquals(-1, socket.getInputStream().read());
            }
            echoed.get(10, TimeUnit.SECONDS);
            client.command(json("op", "stop", "id", tunnel.getString("id")));
            assertEquals(0, ((JSONObject)client.command(json("op", "status"))).getJSONArray("tunnels").length());
            server.command(json("op", "revoke", "peer", clientId));
            assertEquals(0, ((JSONObject)server.command(json("op", "status"))).getJSONArray("peers").length());
        } finally { workers.shutdownNow(); }
    }
}
