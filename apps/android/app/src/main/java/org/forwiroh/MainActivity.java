package org.forwiroh;

import android.Manifest;
import android.app.*;
import android.content.*;
import android.net.VpnService;
import android.os.*;
import android.text.InputType;
import android.util.Base64;
import android.view.*;
import android.widget.*;
import com.google.zxing.BarcodeFormat;
import com.google.zxing.integration.android.IntentIntegrator;
import com.google.zxing.integration.android.IntentResult;
import com.journeyapps.barcodescanner.BarcodeEncoder;
import org.json.*;
import java.nio.charset.StandardCharsets;
import static org.forwiroh.NetworkService.json;

/** Native controls; all networking and identity work is performed by the service. */
public final class MainActivity extends Activity {
    private static final int VPN_PERMISSION = 100;
    private NetworkService service;
    private boolean bound, refreshing;
    private LinearLayout peers, tunnels, pending;
    private TextView state, failure;
    private String vpnPeer, vpnRoutes;
    private final Handler timer = new Handler(Looper.getMainLooper());
    private final Runnable poll = new Runnable() {
        @Override public void run() { refresh(); timer.postDelayed(this, 2000); }
    };
    private final ServiceConnection connection = new ServiceConnection() {
        @Override public void onServiceConnected(ComponentName name, IBinder binder) {
            service = ((NetworkService.LocalBinder)binder).service();
            service.observe(MainActivity.this::refresh); refresh();
        }
        @Override public void onServiceDisconnected(ComponentName name) { service = null; refresh(); }
    };
    @Override public void onCreate(Bundle saved) {
        super.onCreate(saved);
        if (saved != null) { vpnPeer = saved.getString("vpnPeer"); vpnRoutes = saved.getString("vpnRoutes"); }
        LinearLayout root = column(); root.setPadding(dp(20), dp(12), dp(20), dp(16));
        root.setOnApplyWindowInsetsListener((v, insets) -> {
            v.setPadding(dp(20) + insets.getSystemWindowInsetLeft(), dp(12) + insets.getSystemWindowInsetTop(),
                    dp(20) + insets.getSystemWindowInsetRight(), dp(16) + insets.getSystemWindowInsetBottom());
            return insets;
        });
        ScrollView scroll = new ScrollView(this); scroll.setFillViewport(true); scroll.addView(root); setContentView(scroll);
        title(root, "Forwiroh"); text(root, "Private connections to your devices");
        state = text(root, "Starting networking…"); failure = text(root, ""); failure.setTextColor(0xffa12323);
        button(root, "Start networking", this::startNetworking);
        button(root, "Stop all networking", () -> new AlertDialog.Builder(this).setTitle("Stop networking?")
                .setMessage("All socket tunnels and VPN will close. Paired devices remain saved.")
                .setNegativeButton("Cancel", null).setPositiveButton("Stop", (d,w) -> {
                    if (service != null) service.observe(null);
                    if (bound) { unbindService(connection); bound = false; }
                    service = null; stopService(new Intent(this, NetworkService.class)); refresh();
                }).show());
        title(root, "Pair devices");
        button(root, "Scan invitation QR", () -> new IntentIntegrator(this).setDesiredBarcodeFormats(IntentIntegrator.QR_CODE)
                .setPrompt("Scan a Forwiroh invitation").setBeepEnabled(false).setOrientationLocked(false).initiateScan());
        button(root, "Paste invitation", () -> {
            EditText uri = field("forwiroh://pair?data=…", "");
            new AlertDialog.Builder(this).setTitle("Pair a device").setView(uri).setNegativeButton("Cancel", null)
                    .setPositiveButton("Review", (d,w) -> reviewPair(uri.getText().toString().trim())).show();
        });
        button(root, "Show invitation QR", () -> request(json("op", "invite"), data -> showInvite((JSONObject)data)));
        button(root, "Cancel outgoing pairing", () -> run(json("op", "cancel_pair")));
        pending = column(); root.addView(pending);
        title(root, "Peers"); peers = column(); root.addView(peers);
        title(root, "Active tunnels"); tunnels = column(); root.addView(tunnels);
        button(root, "Stop VPN", () -> { if (service != null) service.stopVpn(); });
        text(root, "Socket tunnels run while the networking notification is active. VPN supports private IPv4 split routes only; configure the peer’s VPN policy and Linux TUN first. No full tunnel, IPv6 VPN or SOCKS UDP.");
        title(root, "About Forwiroh");
        text(root, "Android " + BuildConfig.VERSION_NAME + " (" + BuildConfig.VERSION_CODE + ") · Rust " + RustCore.version());
        startNetworking();
        if (Build.VERSION.SDK_INT >= 33 && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != android.content.pm.PackageManager.PERMISSION_GRANTED)
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 101);
    }
    private void startNetworking() {
        if (bound && service != null && service.stopped()) {
            unbindService(connection); bound = false; service = null; refreshing = false;
        }
        if (bound) return;
        Intent intent = new Intent(this, NetworkService.class);
        try { startForegroundService(intent); bound = bindService(intent, connection, 0); }
        catch (RuntimeException e) { error(e.getMessage()); }
    }
    @Override protected void onStart() { super.onStart(); timer.post(poll); }
    @Override protected void onStop() { timer.removeCallbacks(poll); super.onStop(); }
    @Override protected void onDestroy() {
        if (service != null) service.observe(null);
        if (bound) unbindService(connection);
        super.onDestroy();
    }
    @Override protected void onSaveInstanceState(Bundle out) {
        super.onSaveInstanceState(out); out.putString("vpnPeer", vpnPeer); out.putString("vpnRoutes", vpnRoutes);
    }
    private interface Reply { void receive(Object data) throws Exception; }
    private void request(JSONObject command, Reply reply) {
        if (service == null) { error("Start networking first"); return; }
        error("");
        NetworkService source = service;
        source.request(command, (data, message) -> {
            if (isDestroyed() || service != source || source.stopped()) return;
            if (message != null) { error(message); return; }
            try { reply.receive(data); } catch (Exception e) { error(e.getMessage()); }
        });
    }
    private void run(JSONObject command) { request(command, data -> refresh()); }
    private void error(String message) { failure.setText(message == null ? "Unknown error" : message); }
    private void refresh() {
        String serviceError = service == null ? null : service.takeError();
        if (serviceError != null) error(serviceError);
        if (service == null || !service.ready()) { state.setText("Networking stopped or starting…"); peers.removeAllViews(); tunnels.removeAllViews(); pending.removeAllViews(); return; }
        if (refreshing) return;
        refreshing = true;
        NetworkService source = service;
        source.request(json("op", "status"), (data, message) -> {
            refreshing = false;
            if (isDestroyed() || service != source) return;
            if (message != null) { error(message); return; }
            try {
                JSONObject status = (JSONObject)data;
                state.setText("Networking active" + (source.vpnId() == null ? "" : " · VPN active") + "\n" + status.getString("id"));
                peers.removeAllViews();
                JSONArray list = status.getJSONArray("peers");
                if (list.length() == 0) text(peers, "No paired devices yet. Scan your server’s invitation to begin.");
                for (int i = 0; i < list.length(); i++) peerRow(list.getJSONObject(i));
                tunnels.removeAllViews(); list = status.getJSONArray("tunnels");
                if (list.length() == 0) text(tunnels, "No active socket tunnels");
                for (int i = 0; i < list.length(); i++) {
                    JSONObject t = list.getJSONObject(i);
                    text(tunnels, t.getString("kind") + " · " + t.getString("detail"));
                    button(tunnels, "Stop tunnel", () -> run(json("op", "stop", "id", t.optString("id"))));
                }
            } catch (Exception e) { error(e.getMessage()); }
        });
        source.request(json("op", "pending"), (data, message) -> {
            if (isDestroyed() || service != source || message != null) return;
            pending.removeAllViews();
            try {
                JSONArray requests = (JSONArray)data;
                for (int i = 0; i < requests.length(); i++) {
                    JSONObject p = requests.getJSONObject(i);
                    text(pending, "Pairing request: " + p.getString("name") + " (" + p.getString("device") + ")\n" + p.getString("peer"));
                    button(pending, "Review request", () -> new AlertDialog.Builder(this).setTitle("Trust this device?")
                            .setMessage(p.optString("name") + "\n" + p.optString("peer") + "\nVerify this identity with the other device. Pairing grants TCP access unless restricted.")
                            .setNegativeButton("Reject", (d,w) -> run(json("op", "approve", "id", p.optString("id"), "accept", false)))
                            .setPositiveButton("Trust", (d,w) -> run(json("op", "approve", "id", p.optString("id"), "accept", true))).show());
                }
            } catch (Exception e) { error(e.getMessage()); }
        });
    }
    private void peerRow(JSONObject peer) throws Exception {
        String id = peer.getString("id");
        text(peers, peer.getString("name") + " · " + (peer.optBoolean("connected") ? peer.optString("path", "Connected") : "Disconnected"));
        button(peers, "Manage " + peer.getString("name"), () -> new AlertDialog.Builder(this)
                .setTitle(peer.optString("name")).setItems(new String[]{"TCP forward", "Reverse forward", "SOCKS5", "VPN", "Connect / services", "Rename", "Edit inbound policy", "Disconnect", "Revoke trust"}, (d, which) -> {
                    switch (which) {
                        case 0: case 1: case 2: tunnelDialog(id, which); break;
                        case 3: vpnDialog(id); break;
                        case 4: request(json("op", "services", "peer", id), data -> {
                            new AlertDialog.Builder(this).setTitle("Peer services").setMessage(data.toString()).setPositiveButton("OK", null).show(); refresh();
                        }); break;
                        case 5: {
                            EditText name = field("Peer name", peer.optString("name"));
                            new AlertDialog.Builder(this).setTitle("Rename peer").setView(name).setNegativeButton("Cancel", null)
                                    .setPositiveButton("Save", (x,w) -> run(json("op", "rename", "peer", id, "name", name.getText().toString().trim()))).show(); break;
                        }
                        case 6: policyDialog(peer); break;
                        case 7: run(json("op", "disconnect", "peer", id)); break;
                        case 8: new AlertDialog.Builder(this).setTitle("Revoke trust?").setMessage(id + "\nAll sessions with this peer will close.")
                                .setNegativeButton("Cancel", null).setPositiveButton("Revoke", (x,w) -> run(json("op", "revoke", "peer", id))).show(); break;
                        default: break;
                    }
                }).show());
    }
    private void tunnelDialog(String peer, int kind) {
        LinearLayout form = column();
        EditText port = field(kind == 1 ? "Peer listening port" : "Local listening port", kind == 2 ? "1080" : "2222");
        port.setInputType(InputType.TYPE_CLASS_NUMBER); form.addView(port);
        EditText destination = field(kind == 1 ? "Local numeric IP:port" : "Peer service name or IP:port", "127.0.0.1:22");
        if (kind != 2) form.addView(destination);
        CheckBox lan = new CheckBox(this); lan.setText("Listen on all interfaces (exposes this port to your LAN)");
        if (kind == 0) form.addView(lan);
        AlertDialog dialog = new AlertDialog.Builder(this).setTitle(kind == 0 ? "TCP forward" : kind == 1 ? "Reverse forward" : "SOCKS5")
                .setView(form).setNegativeButton("Cancel", null).setPositiveButton("Start", null).create();
        dialog.setOnShowListener(d -> dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
            try {
                int number = Integer.parseInt(port.getText().toString());
                if (number < 1 || number > 65535) throw new IllegalArgumentException("Port must be 1–65535");
                JSONObject command = json("op", kind == 0 ? "forward" : kind == 1 ? "reverse" : "socks", "peer", peer);
                if (kind == 1) { command.put("port", number); command.put("target", destination.getText().toString().trim()); }
                else {
                    command.put("listen", (kind == 0 && lan.isChecked() ? "0.0.0.0:" : "127.0.0.1:") + number);
                    if (kind == 0) command.put("service", destination.getText().toString().trim());
                }
                request(command, data -> { dialog.dismiss(); refresh(); });
            } catch (Exception e) { port.setError(e.getMessage()); }
        })); dialog.show();
    }
    private void policyDialog(JSONObject peer) {
        EditText policy = field("Inbound policy JSON", peer.optJSONObject("policy").toString());
        policy.setSingleLine(false); policy.setMinLines(8);
        AlertDialog dialog = new AlertDialog.Builder(this).setTitle("Access to this Android device")
                .setMessage("Advanced: restricted=true enables grants. Saving closes existing sessions. VPN serving requires Linux.")
                .setView(policy).setNegativeButton("Cancel", null).setPositiveButton("Save", null).create();
        dialog.setOnShowListener(d -> dialog.getButton(AlertDialog.BUTTON_POSITIVE).setOnClickListener(v -> {
            try { request(json("op", "policy", "peer", peer.getString("id"), "policy", new JSONObject(policy.getText().toString())), data -> { dialog.dismiss(); refresh(); }); }
            catch (Exception e) { policy.setError(e.getMessage()); }
        })); dialog.show();
    }
    private void reviewPair(String uri) {
        try {
            String prefix = "forwiroh://pair?data=";
            if (uri.length() > 4096 || !uri.startsWith(prefix)) throw new IllegalArgumentException("Not a Forwiroh invitation");
            JSONObject invitation = new JSONObject(new String(Base64.decode(uri.substring(prefix.length()), Base64.URL_SAFE | Base64.NO_WRAP | Base64.NO_PADDING), StandardCharsets.UTF_8));
            new AlertDialog.Builder(this).setTitle("Pair with " + invitation.getString("name") + "?")
                    .setMessage("Only accept invitations from a device you trust. Paired peers can access TCP destinations on this device unless you restrict their policy. Confirm this request on the other device.")
                    .setNegativeButton("Cancel", null).setPositiveButton("Pair", (d,w) -> {
                        state.setText("Waiting for approval on the other device…");
                        run(json("op", "pair", "uri", uri, "device", "Android"));
                    }).show();
        } catch (Exception e) { error(e.getMessage()); }
    }
    private void showInvite(JSONObject invitation) throws Exception {
        String uri = invitation.getString("uri");
        LinearLayout content = column();
        ImageView image = new ImageView(this); image.setContentDescription("Forwiroh pairing invitation QR");
        image.setImageBitmap(new BarcodeEncoder().encodeBitmap(uri, BarcodeFormat.QR_CODE, 768, 768));
        content.addView(image, new LinearLayout.LayoutParams(-1, dp(280)));
        text(content, "Expires in five minutes. After scanning, tap Review requests to approve the requesting identity.");
        NetworkService owner = service;
        Runnable cancel = () -> owner.request(json("op", "cancel_invite", "id", invitation.optString("id")), (data, message) -> { });
        AlertDialog dialog = new AlertDialog.Builder(this).setTitle("Pair another device").setView(content)
                .setPositiveButton("Review requests", (d,w) -> refresh())
                .setNegativeButton("Cancel invitation", (d,w) -> cancel.run()).create();
        dialog.setOnCancelListener(d -> cancel.run());
        dialog.show();
    }
    private void vpnDialog(String peer) {
        EditText routes = field("Private IPv4 CIDRs, separated by commas", "192.168.1.0/24");
        new AlertDialog.Builder(this).setTitle("Start split-route VPN").setView(routes)
                .setMessage("Only these private IPv4 networks use the peer. Other traffic and DNS keep the system network. Forwiroh itself bypasses VPN.")
                .setNegativeButton("Cancel", null).setPositiveButton("Start", (d,w) -> {
                    vpnPeer = peer; vpnRoutes = routes.getText().toString();
                    Intent consent = VpnService.prepare(this);
                    if (consent == null) startVpn(); else startActivityForResult(consent, VPN_PERMISSION);
                }).show();
    }
    private void startVpn() {
        if (service == null || vpnPeer == null || vpnRoutes == null) { error("Select a VPN peer first"); return; }
        JSONArray routes = new JSONArray();
        for (String route : vpnRoutes.split(",")) routes.put(route.trim());
        service.startVpn(vpnPeer, routes, (data, message) -> { if (!isDestroyed()) { if (message != null) error(message); refresh(); } });
    }
    @Override protected void onActivityResult(int request, int result, Intent data) {
        super.onActivityResult(request, result, data);
        if (request == VPN_PERMISSION) { if (result == RESULT_OK) startVpn(); else error("VPN permission was declined"); return; }
        IntentResult scan = IntentIntegrator.parseActivityResult(request, result, data);
        if (scan != null && scan.getContents() != null) reviewPair(scan.getContents().trim());
    }
    private int dp(int size) { return Math.round(size * getResources().getDisplayMetrics().density); }
    private LinearLayout column() { LinearLayout view = new LinearLayout(this); view.setOrientation(LinearLayout.VERTICAL); return view; }
    private TextView text(LinearLayout parent, String value) {
        TextView view = new TextView(this); view.setText(value); view.setTextSize(16); view.setPadding(0, dp(5), 0, dp(5)); view.setTextIsSelectable(true); parent.addView(view); return view;
    }
    private void title(LinearLayout parent, String value) { TextView view = text(parent, value); view.setTextSize(24); view.setPadding(0, dp(20), 0, dp(8)); }
    private void button(LinearLayout parent, String title, Runnable action) {
        Button button = new Button(this); button.setText(title); button.setAllCaps(false); button.setOnClickListener(v -> action.run()); parent.addView(button);
    }
    private EditText field(String hint, String value) {
        EditText input = new EditText(this); input.setHint(hint); input.setContentDescription(hint); input.setText(value); input.setSingleLine(true);
        input.setInputType(InputType.TYPE_CLASS_TEXT | InputType.TYPE_TEXT_FLAG_NO_SUGGESTIONS); return input;
    }
}
