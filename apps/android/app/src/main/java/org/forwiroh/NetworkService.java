package org.forwiroh;

import android.app.*;
import android.content.Intent;
import android.net.VpnService;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.LinkProperties;
import android.os.*;
import android.system.OsConstants;
import org.json.*;
import java.io.*;
import java.util.Arrays;
import java.util.concurrent.*;

/** One endpoint owner for the UI, background socket tunnels and optional VPN. */
public final class NetworkService extends VpnService {
    static final String STOP = "org.forwiroh.STOP";
    private final Handler main = new Handler(Looper.getMainLooper());
    private final ExecutorService lifecycle = Executors.newSingleThreadExecutor();
    private final ThreadPoolExecutor commands = new ThreadPoolExecutor(3, 3, 0, TimeUnit.SECONDS,
            new ArrayBlockingQueue<>(32));
    private final LocalBinder binder = new LocalBinder();
    private volatile RustCore core;
    private volatile boolean stopping;
    private volatile String error;
    private volatile String vpnId;
    private volatile boolean pumping;
    private ParcelFileDescriptor tun;
    private FileOutputStream tunOutput;
    private Thread outgoing, incoming;
    private Runnable observer;
    private final Runnable notifyNetwork = () -> {
        if (ready()) request(json("op", "network_change"), (data, failure) -> { });
    };
    private final ConnectivityManager.NetworkCallback networkChanges = new ConnectivityManager.NetworkCallback() {
        private void changedNetwork() { main.removeCallbacks(notifyNetwork); main.postDelayed(notifyNetwork, 250); }
        @Override public void onAvailable(Network network) { changedNetwork(); }
        @Override public void onLost(Network network) { changedNetwork(); }
        @Override public void onLinkPropertiesChanged(Network network, LinkProperties properties) { changedNetwork(); }
    };

    public final class LocalBinder extends Binder { NetworkService service() { return NetworkService.this; } }
    interface Result { void done(Object data, String error); }
    static JSONObject json(Object... pairs) {
        JSONObject value = new JSONObject();
        try { for (int i = 0; i < pairs.length; i += 2) value.put((String)pairs[i], pairs[i + 1]); }
        catch (JSONException e) { throw new IllegalArgumentException(e); }
        return value;
    }
    @Override public IBinder onBind(Intent intent) {
        return SERVICE_INTERFACE.equals(intent.getAction()) ? super.onBind(intent) : binder;
    }
    @Override public void onCreate() {
        super.onCreate();
        NotificationManager notifications = getSystemService(NotificationManager.class);
        notifications.createNotificationChannel(new NotificationChannel("network", "Active networking", NotificationManager.IMPORTANCE_LOW));
        PendingIntent open = PendingIntent.getActivity(this, 0, new Intent(this, MainActivity.class), PendingIntent.FLAG_IMMUTABLE);
        PendingIntent stop = PendingIntent.getService(this, 1, new Intent(this, NetworkService.class).setAction(STOP), PendingIntent.FLAG_IMMUTABLE);
        startForeground(1, new Notification.Builder(this, "network").setSmallIcon(R.drawable.ic_network)
                .setContentTitle("Forwiroh networking active").setContentText("Paired peers and tunnels remain available")
                .setContentIntent(open).setOngoing(true).addAction(new Notification.Action.Builder(null, "Stop", stop).build()).build());
        getSystemService(ConnectivityManager.class).registerDefaultNetworkCallback(networkChanges, main);
        lifecycle.execute(() -> {
            try {
                byte[] key = Identity.load(this);
                try {
                    for (int attempt = 0; attempt < 6; attempt++) {
                        if (stopping) return;
                        try { core = new RustCore(this, new File(getNoBackupFilesDir(), "state").getPath(), key); break; }
                        catch (IllegalStateException e) {
                            // A previous service may still be releasing its endpoint after Stop.
                            if (attempt == 5 || !String.valueOf(e.getMessage()).contains("state is already in use")) throw e;
                            Thread.sleep(100L << attempt);
                        }
                    }
                }
                finally { Arrays.fill(key, (byte)0); }
                new Thread(() -> {
                    try {
                        while (!stopping) {
                            JSONObject event = core.event();
                            if (event == null) break;
                            if ("access_request".equals(event.optString("event"))) android.util.Log.i("Forwiroh", event.toString());
                            if (event.has("error")) error = event.optString("error");
                            changed();
                        }
                    } catch (Exception e) { if (!stopping) report(e); }
                }, "forwiroh-events").start();
                changed();
            } catch (Exception e) { report(e); main.post(() -> { shutdown(); stopSelf(); }); }
        });
    }
    @Override public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && STOP.equals(intent.getAction())) { shutdown(); stopSelf(); }
        return START_NOT_STICKY; // Never silently recreate tunnels or VPN after process death.
    }
    void observe(Runnable listener) { observer = listener; }
    boolean ready() { return core != null && !stopping; }
    String takeError() { String value = error; error = null; return value; }
    boolean stopped() { return stopping; }
    String vpnId() { return vpnId; }
    private void changed() { main.post(() -> { if (observer != null) observer.run(); }); }
    private void report(Exception e) { error = e.getMessage() == null ? e.toString() : e.getMessage(); changed(); }
    void request(JSONObject command, Result result) {
        if (!ready()) { result.done(null, "Networking is not ready"); return; }
        try {
            commands.execute(() -> {
                Object data = null; String failure = null;
                try { data = core.command(command); } catch (Exception e) { failure = e.getMessage(); }
                Object reply = data; String message = failure;
                main.post(() -> result.done(reply, message));
            });
        } catch (RejectedExecutionException e) { result.done(null, "Too many pending requests"); }
    }
    void startVpn(String peer, JSONArray routes, Result result) {
        if (!ready()) { result.done(null, "Networking is not ready"); return; }
        lifecycle.execute(() -> {
            try {
                if (stopping) throw new IllegalStateException("Networking is stopped");
                if (vpnId != null) throw new IllegalStateException("Stop the current VPN first");
                if (VpnService.prepare(this) != null) throw new IllegalStateException("VPN permission is required");
                JSONObject info = (JSONObject)core.command(json("op", "vpn_start", "peer", peer, "routes", routes));
                vpnId = info.getString("id");
                try {
                    Builder builder = new Builder().setSession("Forwiroh").setMtu(info.getInt("mtu"))
                            .addAddress(info.getString("local_ip"), 32)
                            .allowFamily(OsConstants.AF_INET6) // IPv6 remains on the underlying network.
                            // Keep every core socket outside VPN, including private peer endpoints.
                            .addDisallowedApplication(getPackageName());
                    JSONArray allowed = info.getJSONArray("routes");
                    for (int i = 0; i < allowed.length(); i++) {
                        String[] route = allowed.getString(i).split("/");
                        builder.addRoute(route[0], Integer.parseInt(route[1]));
                    }
                    tun = builder.establish();
                    if (tun == null) throw new IllegalStateException("VPN permission was revoked");
                    tunOutput = new ParcelFileDescriptor.AutoCloseOutputStream(ParcelFileDescriptor.dup(tun.getFileDescriptor()));
                    String session = vpnId;
                    pumping = true;
                    outgoing = new Thread(() -> pumpOutgoing(session), "forwiroh-tun-read");
                    incoming = new Thread(() -> pumpIncoming(session), "forwiroh-tun-write");
                    outgoing.start(); incoming.start();
                } catch (Exception e) { stopVpnNow(); throw e; }
                main.post(() -> result.done(null, null));
                changed();
            } catch (Exception e) { main.post(() -> result.done(null, e.getMessage())); }
        });
    }
    private void pumpOutgoing(String session) {
        try {
            byte[] packet = new byte[65535];
            while (pumping) {
                int n = RustCore.readTun(tun.getFd(), packet);
                if (n < 0) throw new IOException("VPN interface closed");
                // Policy/MTU rejects drop a packet; they do not tear down the interface.
                if (n > 0 && n <= 1100 && pumping) core.send(session, packet, n);
            }
        } catch (Exception e) { if (pumping) vpnFailed(session, e); }
    }
    private void pumpIncoming(String session) {
        try {
            while (pumping) {
                byte[] packet = core.receive(session);
                if (packet == null) throw new IOException("VPN peer disconnected; reconnect to start a new session");
                if (pumping) tunOutput.write(packet);
            }
        } catch (Exception e) { if (pumping) vpnFailed(session, e); }
    }
    private void vpnFailed(String session, Exception e) {
        if (stopping) return;
        try { lifecycle.execute(() -> { if (session.equals(vpnId)) { report(e); stopVpnNow(); } }); }
        catch (RejectedExecutionException ignored) { /* shutdown already owns cleanup */ }
    }
    void stopVpn() {
        if (stopping) return;
        try { lifecycle.execute(this::stopVpnNow); }
        catch (RejectedExecutionException ignored) { /* revocation raced with service shutdown */ }
    }
    private void stopVpnNow() {
        pumping = false;
        if (vpnId != null) {
            try { core.command(json("op", "stop_vpn", "id", vpnId)); } catch (Exception e) { if (!stopping) report(e); }
        }
        join(outgoing); join(incoming);
        outgoing = null; incoming = null;
        try { if (tunOutput != null) tunOutput.close(); } catch (IOException ignored) { }
        try { if (tun != null) tun.close(); } catch (IOException ignored) { }
        tunOutput = null; tun = null; vpnId = null;
        changed();
    }
    private static void join(Thread thread) {
        if (thread == null) return;
        boolean interrupted = false;
        while (thread.isAlive()) {
            try { thread.join(); } catch (InterruptedException e) { interrupted = true; }
        }
        if (interrupted) Thread.currentThread().interrupt();
    }
    @Override public void onRevoke() { stopVpn(); }
    private void shutdown() {
        if (stopping) return;
        stopping = true;
        getSystemService(ConnectivityManager.class).unregisterNetworkCallback(networkChanges);
        main.removeCallbacks(notifyNetwork);
        observer = null;
        commands.shutdownNow();
        // Wake a pairing/VPN negotiation already blocking the lifecycle executor.
        new Thread(() -> { RustCore current = core; if (current != null) current.close(); }, "forwiroh-shutdown").start();
        lifecycle.execute(() -> {
            stopVpnNow();
            if (core != null) core.close();
            core = null;
        });
        lifecycle.shutdown();
        stopForeground(STOP_FOREGROUND_REMOVE);
    }
    @Override public void onDestroy() { shutdown(); super.onDestroy(); }
}
