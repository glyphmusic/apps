package app.glyph;

import android.content.ClipData;
import android.Manifest;
import android.app.PendingIntent;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInstaller;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.provider.Settings;
import android.util.Log;
import android.view.Display;
import android.view.View;

import java.io.File;
import java.io.FileInputStream;
import java.io.InputStream;
import java.io.OutputStream;

import androidx.core.graphics.Insets;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;

import com.google.androidgamesdk.GameActivity;

/**
 * Hosts the Rust app (libglyph_android.so, android_main). Everything on
 * screen is drawn by GPUI; this class only reports insets and offers the
 * few SDK calls Rust makes over JNI (the gpui* methods).
 */
public class MainActivity extends GameActivity {

    private native void nativeInsets(int top, int right, int bottom, int left,
                                     int imeTop, int imeRight, int imeBottom, int imeLeft);

    /** A glyph:// link is waiting; Rust takes it with gpuiTakeOpenUrl. */
    private native void nativeUrlWaiting();

    /** Link the app was opened with (jam invite, playlist), until Rust takes it. */
    private String openUrl;

    /** Result of a PackageInstaller session, delivered back to this activity. */
    private static final String ACTION_INSTALL_STATUS = "app.glyph.INSTALL_STATUS";

    /** Downloaded update waiting for the "install unknown apps" permission. */
    private String pendingApk;

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        // Draw behind the status and navigation bars; GPUI keeps content in
        // the safe area from the insets below.
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        super.onCreate(savedInstanceState);
        takeLink(getIntent());
        // The playback notification needs this on Android 13+.
        if (android.os.Build.VERSION.SDK_INT >= 33
                && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 1);
        }
    }

    @Override
    public WindowInsetsCompat onApplyWindowInsets(View v, WindowInsetsCompat insets) {
        Insets safe = insets.getInsets(WindowInsetsCompat.Type.systemBars() | WindowInsetsCompat.Type.displayCutout());
        Insets ime = insets.getInsets(WindowInsetsCompat.Type.ime());
        nativeInsets(safe.top, safe.right, safe.bottom, safe.left, ime.top, ime.right, ime.bottom, ime.left);
        return super.onApplyWindowInsets(v, insets);
    }

    /** singleTask: a link opened while the app runs arrives here. */
    @Override
    protected void onNewIntent(Intent intent) {
        super.onNewIntent(intent);
        if (ACTION_INSTALL_STATUS.equals(intent.getAction())) {
            installStatus(intent);
            return;
        }
        setIntent(intent);
        takeLink(intent);
    }

    /** Back from the permission screen: go on with the update once allowed. */
    @Override
    protected void onResume() {
        super.onResume();
        if (pendingApk != null && getPackageManager().canRequestPackageInstalls()) {
            String path = pendingApk;
            pendingApk = null;
            installApk(path);
        }
    }

    /** Writes the APK into an installer session; the system then asks the user. */
    private void installApk(String path) {
        new Thread(() -> {
            File apk = new File(path);
            try {
                PackageInstaller installer = getPackageManager().getPackageInstaller();
                PackageInstaller.SessionParams params =
                        new PackageInstaller.SessionParams(PackageInstaller.SessionParams.MODE_FULL_INSTALL);
                params.setAppPackageName(getPackageName());
                int id = installer.createSession(params);
                try (PackageInstaller.Session session = installer.openSession(id)) {
                    try (InputStream in = new FileInputStream(apk);
                         OutputStream out = session.openWrite("glyph.apk", 0, apk.length())) {
                        byte[] buf = new byte[1 << 16];
                        int n;
                        while ((n = in.read(buf)) > 0) {
                            out.write(buf, 0, n);
                        }
                        session.fsync(out);
                    }
                    // Mutable: the installer adds the status extras.
                    int flags = PendingIntent.FLAG_UPDATE_CURRENT
                            | (Build.VERSION.SDK_INT >= 31 ? PendingIntent.FLAG_MUTABLE : 0);
                    Intent status = new Intent(this, MainActivity.class).setAction(ACTION_INSTALL_STATUS);
                    session.commit(PendingIntent.getActivity(this, 0, status, flags).getIntentSender());
                }
            } catch (Exception e) {
                Log.w("glyph", "installing the update failed", e);
            } finally {
                apk.delete();
            }
        }).start();
    }

    @SuppressWarnings("deprecation")
    private void installStatus(Intent intent) {
        int status = intent.getIntExtra(PackageInstaller.EXTRA_STATUS, PackageInstaller.STATUS_FAILURE);
        if (status == PackageInstaller.STATUS_PENDING_USER_ACTION) {
            Intent confirm = intent.getParcelableExtra(Intent.EXTRA_INTENT);
            if (confirm != null) {
                startActivity(confirm);
            }
        } else if (status != PackageInstaller.STATUS_SUCCESS) {
            Log.w("glyph", "update not installed: " + intent.getStringExtra(PackageInstaller.EXTRA_STATUS_MESSAGE));
        }
    }

    private void takeLink(Intent intent) {
        if (intent == null || !Intent.ACTION_VIEW.equals(intent.getAction()) || intent.getData() == null) {
            return;
        }
        synchronized (this) {
            openUrl = intent.getData().toString();
        }
        nativeUrlWaiting();
    }

    /** Back that the app did not take: keep running (and playing) in the background. */
    @Override
    @SuppressWarnings("deprecation")
    public void onBackPressed() {
        moveTaskToBack(true);
    }

    // ------------------------------------------------------------ called from Rust

    public void gpuiOpenUrl(String url) {
        runOnUiThread(() -> {
            try {
                startActivity(new Intent(Intent.ACTION_VIEW, Uri.parse(url)));
            } catch (Exception ignored) {
            }
        });
    }

    /**
     * Installs a downloaded, verified update. Without the "install unknown
     * apps" permission it opens that setting first and goes on in onResume.
     */
    public void gpuiInstallApk(String path) {
        runOnUiThread(() -> {
            if (!getPackageManager().canRequestPackageInstalls()) {
                pendingApk = path;
                try {
                    startActivity(new Intent(Settings.ACTION_MANAGE_UNKNOWN_APP_SOURCES,
                            Uri.parse("package:" + getPackageName())));
                } catch (Exception ignored) {
                }
                return;
            }
            installApk(path);
        });
    }

    public void gpuiSetClipboard(String text) {
        ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
        if (cm != null) {
            cm.setPrimaryClip(ClipData.newPlainText("Glyph", text));
        }
    }

    public String gpuiGetClipboard() {
        ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
        if (cm == null || !cm.hasPrimaryClip() || cm.getPrimaryClip() == null || cm.getPrimaryClip().getItemCount() == 0) {
            return null;
        }
        CharSequence text = cm.getPrimaryClip().getItemAt(0).coerceToText(this);
        return text == null ? null : text.toString();
    }

    /**
     * Now playing from Rust: drives the background playback service. The
     * cover is a cached image file ("" for none); liked: the song is in
     * Liked Songs (the notification's add button shows a check).
     */
    public void gpuiNowPlaying(String title, String artist, boolean playing, long positionMs, long durationMs,
                               String coverPath, boolean liked) {
        runOnUiThread(() -> PlaybackService.update(this, title, artist, playing, positionMs, durationMs, coverPath, liked));
    }

    /** The link the app was opened with, once (null when there is none). */
    public synchronized String gpuiTakeOpenUrl() {
        String url = openUrl;
        openUrl = null;
        return url;
    }

    public float gpuiRefreshRate() {
        Display d = getWindowManager().getDefaultDisplay();
        return d == null ? 60f : d.getRefreshRate();
    }
}
