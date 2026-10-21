package app.glyph;

import android.content.ClipData;
import android.Manifest;
import android.content.ClipboardManager;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Bundle;
import android.view.Display;
import android.view.View;

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
        setIntent(intent);
        takeLink(intent);
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

    /** Now playing from Rust: drives the background playback service. */
    public void gpuiNowPlaying(String title, String artist, boolean playing, long positionMs, long durationMs) {
        runOnUiThread(() -> PlaybackService.update(this, title, artist, playing, positionMs, durationMs));
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
