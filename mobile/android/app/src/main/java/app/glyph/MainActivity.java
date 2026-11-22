package app.glyph;

import android.Manifest;
import android.app.ActivityManager;
import android.app.PendingIntent;
import android.content.ClipData;
import android.content.ClipboardManager;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageInstaller;
import android.content.pm.PackageManager;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.graphics.Color;
import android.hardware.Sensor;
import android.hardware.SensorEvent;
import android.hardware.SensorEventListener;
import android.hardware.SensorManager;
import android.net.ConnectivityManager;
import android.net.Network;
import android.net.NetworkCapabilities;
import android.net.Uri;
import android.os.Build;
import android.os.Bundle;
import android.os.Handler;
import android.os.Looper;
import android.provider.Settings;
import android.util.Log;
import android.view.Display;
import android.view.KeyEvent;
import android.view.View;
import android.view.ViewGroup;
import android.view.ViewParent;
import android.webkit.JavascriptInterface;
import android.webkit.RenderProcessGoneDetail;
import android.webkit.ValueCallback;
import android.webkit.WebChromeClient;
import android.webkit.WebResourceRequest;
import android.webkit.WebResourceResponse;
import android.webkit.WebSettings;
import android.webkit.WebView;
import android.webkit.WebViewClient;

import androidx.activity.OnBackPressedCallback;
import androidx.activity.result.ActivityResultLauncher;
import androidx.activity.result.contract.ActivityResultContracts;
import androidx.appcompat.app.AppCompatActivity;
import androidx.core.graphics.Insets;
import androidx.core.view.ViewCompat;
import androidx.core.view.WindowCompat;
import androidx.core.view.WindowInsetsCompat;

import org.json.JSONObject;

import java.io.ByteArrayOutputStream;
import java.io.ByteArrayInputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.Set;
import java.util.concurrent.CountDownLatch;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;
import java.util.concurrent.atomic.AtomicInteger;
import java.util.concurrent.atomic.AtomicReference;

/**
 * Hosts the Svelte UI inside a WebView and bridges it to the Rust core/audio
 * library (libglyph_android.so), while driving Android-specific features:
 * PlaybackService media controls, network status, insets, APK self-updates,
 * deep links, share intents, clipboard, and hardware/remote volume keys.
 */
public class MainActivity extends AppCompatActivity {

    static {
        System.loadLibrary("glyph_android");
    }

    private static final String APP_ORIGIN = "https://glyph.localhost";
    private static final String APP_HOST = "glyph.localhost";
    private static final String ASSET_PREFIX = "/__asset__";

    /** Result of a PackageInstaller session, delivered back to this activity. */
    private static final String ACTION_INSTALL_STATUS = "app.glyph.INSTALL_STATUS";
    private static final int DEFAULT_ACCENT = 0xff5fd2;
    private static final String[] LAUNCHER_THEMES = {
            "glyph", "ember", "ocean", "forest", "violet", "crimson", "mono"
    };
    private static final String[] LAUNCHER_ALIASES = {
            "app.glyph.LauncherGlyph",
            "app.glyph.LauncherEmber",
            "app.glyph.LauncherOcean",
            "app.glyph.LauncherForest",
            "app.glyph.LauncherViolet",
            "app.glyph.LauncherCrimson",
            "app.glyph.LauncherMono"
    };
    private static final int[] LAUNCHER_ACCENTS = {
            0xff5fd2, 0xff8a3d, 0x4da3ff, 0x3ddc84, 0xb07cff, 0xff4d5e, 0xe8e8e8
    };

    private native void nativeInit(MainActivity activity, String filesDir, boolean sheet);
    private native void nativeDetach(MainActivity activity);
    private native void nativeInvoke(long id, String cmd, String argsJson);
    private native void nativeInsets(int top, int right, int bottom, int left,
                                     int imeTop, int imeRight, int imeBottom, int imeLeft);
    /** A glyph:// link is waiting; Rust takes it with gpuiTakeOpenUrl. */
    private native void nativeUrlWaiting();
    private native void nativeConnectivity(boolean online);
    private native void nativeResumed();
    /** The page goes to the background: Rust holds library events until nativeResumed. */
    private native void nativePaused();
    /** Library patches were dropped: Rust sends the whole library again. */
    private native void nativeLibraryResend();
    /** The file chosen in gpuiPickFile (a copy in the cache), or null. */
    private static native void nativeFilePicked(String path);

    private volatile boolean activityPaused;
    /** The page has been in the background (onStop): check it is still alive on return. */
    private boolean checkOnResume;
    /** Bumped for every health check, so an old check's timeout does nothing. */
    private int healthSeq;
    private static final long HEALTH_TIMEOUT_MS = 4000L;
    private static final int MAX_PENDING_EVENTS = 128;
    private static final int MAX_MAIN_FRAME_RETRIES = 3;
    private static final ExecutorService THEME_EXECUTOR = Executors.newSingleThreadExecutor();

    private ConnectivityManager connectivity;
    private ConnectivityManager.NetworkCallback networkCallback;
    private SensorManager sensorManager;
    private Sensor accelerometer;
    private static final float SHAKE_THRESHOLD_G = 1.35f;
    private static final long SHAKE_WINDOW_MS = 900L;
    private static final long SHAKE_MIN_GAP_MS = 110L;
    private static final long SHAKE_COOLDOWN_MS = 2500L;
    private final long[] shakePeaks = new long[3];
    private int shakePeakCount;
    private long lastShakePeakMs;
    private long lastShakeTriggerMs;
    private final SensorEventListener shakeListener = new SensorEventListener() {
        @Override
        public void onSensorChanged(SensorEvent event) {
            if (activityPaused || !pageReady || webView == null) {
                return;
            }
            long now = System.currentTimeMillis();
            if (now - lastShakeTriggerMs < SHAKE_COOLDOWN_MS) {
                return;
            }
            float gx = event.values[0] / SensorManager.GRAVITY_EARTH;
            float gy = event.values[1] / SensorManager.GRAVITY_EARTH;
            float gz = event.values[2] / SensorManager.GRAVITY_EARTH;
            float deltaG = Math.abs((float) Math.sqrt(gx * gx + gy * gy + gz * gz) - 1.0f);
            if (deltaG < SHAKE_THRESHOLD_G || now - lastShakePeakMs < SHAKE_MIN_GAP_MS) {
                return;
            }
            lastShakePeakMs = now;
            int kept = 0;
            for (int i = 0; i < shakePeakCount; i++) {
                if (now - shakePeaks[i] <= SHAKE_WINDOW_MS) {
                    shakePeaks[kept++] = shakePeaks[i];
                }
            }
            shakePeakCount = kept;
            if (shakePeakCount < shakePeaks.length) {
                shakePeaks[shakePeakCount++] = now;
            }
            if (shakePeakCount >= shakePeaks.length) {
                shakePeakCount = 0;
                lastShakeTriggerMs = now;
                runOnUiThread(() -> {
                    if (webView != null && pageReady && !activityPaused) {
                        webView.evaluateJavascript("window.__glyphShake&&window.__glyphShake();", null);
                    }
                });
            }
        }

        @Override
        public void onAccuracyChanged(Sensor sensor, int accuracy) {
        }
    };
    private WebView webView;
    private boolean pageReady;
    private boolean showingFallbackError;
    private boolean mainFrameErrorThisLoad;
    private int mainFrameRetries;
    private int readyPageToken;
    private final AtomicInteger nextPageToken = new AtomicInteger(0);
    private final AtomicInteger latestPageToken = new AtomicInteger(0);
    private String lastInsetsJs;
    private final List<String> pendingInvokes = new ArrayList<>();
    private final List<String> pendingEvents = new ArrayList<>();
    /**
     * Events that only describe the current state: while the page cannot take
     * them (app in the background, page loading) only the newest of each is
     * kept. Before, every library/queue/player update piled up in the frozen
     * page and ran all at once on return, which froze the app.
     */
    private final LinkedHashMap<String, String> pendingLatest = new LinkedHashMap<>();
    private static final Set<String> LATEST_ONLY = new HashSet<>(Arrays.asList(
            "glyph://player", "glyph://queue", "glyph://devices",
            "glyph://config", "glyph://account", "glyph://home", "glyph://jam",
            "glyph://sleep", "glyph://import_progress", "glyph://backend", "glyph://update",
            "glyph://mcp_approvals", "glyph://search"));
    /**
     * Library events that waited: a whole library replaces everything before
     * it, a patch (only the changed parts) must follow the ones before it.
     */
    private final List<String> pendingLibrary = new ArrayList<>();
    private static final int MAX_PENDING_LIBRARY = 64;
    private final Handler themeHandler = new Handler(Looper.getMainLooper());
    private int lastAccentRgb = -1;
    private String pendingAlias;
    private final Runnable applyAliasRunnable = () -> applyLauncherAliasNow(pendingAlias);

    private ValueCallback<Uri[]> fileChooserCallback;
    private final ActivityResultLauncher<Intent> fileChooserLauncher =
            registerForActivityResult(new ActivityResultContracts.StartActivityForResult(), result -> {
                if (fileChooserCallback == null) {
                    return;
                }
                Uri[] uris = WebChromeClient.FileChooserParams.parseResult(result.getResultCode(), result.getData());
                fileChooserCallback.onReceiveValue(uris);
                fileChooserCallback = null;
            });

    /** Link the app was opened with (jam invite, playlist), until Rust takes it. */
    private String openUrl;

    /** Downloaded update waiting for the "install unknown apps" permission. */
    private String pendingApk;

    private void reportConnectivity() {
        if (connectivity == null) {
            return;
        }
        NetworkCapabilities caps = connectivity.getNetworkCapabilities(connectivity.getActiveNetwork());
        nativeConnectivity(caps != null && caps.hasCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET));
    }

    /**
     * The share sheet over another app ({@link ShareActivity}): the same
     * page, see-through, showing only the "Add to Glyph" sheet.
     */
    protected boolean isSheet() {
        return false;
    }

    /** Back with nothing left to close: the app goes to the background, a sheet ends. */
    private void leave() {
        if (isSheet()) {
            closeSheetNow("");
        } else {
            moveTaskToBack(true);
        }
    }

    @Override
    protected void onCreate(Bundle savedInstanceState) {
        WindowCompat.setDecorFitsSystemWindows(getWindow(), false);
        super.onCreate(savedInstanceState);
        if (isSheet()) {
            applyTransition(true, android.R.anim.fade_in, 0);
        }

        nativeInit(this, getFilesDir().getAbsolutePath(), isSheet());

        connectivity = getSystemService(ConnectivityManager.class);
        if (connectivity != null) {
            networkCallback = new ConnectivityManager.NetworkCallback() {
                @Override public void onAvailable(Network network) { reportConnectivity(); }
                @Override public void onLost(Network network) { reportConnectivity(); }
                @Override public void onCapabilitiesChanged(Network network, NetworkCapabilities caps) { reportConnectivity(); }
            };
            connectivity.registerDefaultNetworkCallback(networkCallback);
            reportConnectivity();
        }

        if (!isSheet()) {
            sensorManager = getSystemService(SensorManager.class);
            if (sensorManager != null) {
                accelerometer = sensorManager.getDefaultSensor(Sensor.TYPE_ACCELEROMETER);
            }
        }

        setupWebView();

        getOnBackPressedDispatcher().addCallback(this, new OnBackPressedCallback(true) {
            @Override
            public void handleOnBackPressed() {
                if (webView == null || !pageReady) {
                    leave();
                    return;
                }
                webView.evaluateJavascript(
                        "(function(){return !!(window.__glyphBack&&window.__glyphBack());})()",
                        value -> {
                            if (!"true".equals(value)) {
                                leave();
                            }
                        });
            }
        });

        if (isSheet()) {
            takeShare(getIntent());
            return;
        }
        takeLink(getIntent());

        // The playback notification needs this on Android 13+.
        if (Build.VERSION.SDK_INT >= 33
                && checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            requestPermissions(new String[]{Manifest.permission.POST_NOTIFICATIONS}, 1);
        }
        // New versions and releases also while Glyph is closed (Settings > Notifications).
        Notifications.schedule(getApplicationContext());
    }

    private void setupWebView() {
        WebView view = new WebView(this);
        view.setBackgroundColor(isSheet() ? Color.TRANSPARENT : Color.BLACK);
        view.setLayoutParams(new ViewGroup.LayoutParams(
                ViewGroup.LayoutParams.MATCH_PARENT, ViewGroup.LayoutParams.MATCH_PARENT));

        if ((getApplicationInfo().flags & android.content.pm.ApplicationInfo.FLAG_DEBUGGABLE) != 0) {
            WebView.setWebContentsDebuggingEnabled(true);
        }

        WebSettings settings = view.getSettings();
        settings.setJavaScriptEnabled(true);
        settings.setDomStorageEnabled(true);
        settings.setMediaPlaybackRequiresUserGesture(false);
        settings.setAllowFileAccess(false);
        settings.setAllowContentAccess(false);

        view.addJavascriptInterface(new Bridge(), "__GLYPH_ANDROID__");
        view.setWebChromeClient(new WebChromeClient() {
            @Override
            public boolean onConsoleMessage(android.webkit.ConsoleMessage consoleMessage) {
                Log.d("glyph-web", consoleMessage.message() + " ["
                        + consoleMessage.sourceId() + ":" + consoleMessage.lineNumber() + "]");
                return true;
            }

            @Override
            public boolean onShowFileChooser(WebView wv, ValueCallback<Uri[]> callback, FileChooserParams params) {
                if (fileChooserCallback != null) {
                    fileChooserCallback.onReceiveValue(null);
                }
                fileChooserCallback = callback;
                try {
                    fileChooserLauncher.launch(params.createIntent());
                    return true;
                } catch (Exception e) {
                    fileChooserCallback = null;
                    return false;
                }
            }
        });
        view.setWebViewClient(new WebViewClient() {
            @Override
            public void onReceivedError(WebView wv, WebResourceRequest request, android.webkit.WebResourceError error) {
                if (wv != webView) {
                    return;
                }
                if (request != null && request.isForMainFrame()) {
                    String desc = error != null && error.getDescription() != null
                            ? error.getDescription().toString()
                            : "Unknown error";
                    int code = error != null ? error.getErrorCode() : -1;
                    Log.e("glyph-web", "Main frame error: " + desc + " code=" + code);
                    handleMainFrameFailure("Error " + code + ": " + desc);
                } else if (request != null) {
                    Log.w("glyph-web", "Resource error: " + error.getDescription() + " url=" + request.getUrl());
                }
            }

            @Override
            public void onReceivedHttpError(WebView wv, WebResourceRequest request, WebResourceResponse errorResponse) {
                if (wv != webView) {
                    return;
                }
                if (request != null && request.isForMainFrame() && errorResponse != null
                        && errorResponse.getStatusCode() >= 400) {
                    int status = errorResponse.getStatusCode();
                    Log.e("glyph-web", "Main frame HTTP error: status=" + status + " url=" + request.getUrl());
                    handleMainFrameFailure("HTTP " + status);
                }
            }

            @Override
            public boolean onRenderProcessGone(WebView wv, RenderProcessGoneDetail detail) {
                boolean crashed = detail != null && detail.didCrash();
                Log.e("glyph-web", "WebView renderer gone (didCrash=" + crashed + "); recreating");
                if (wv != webView) {
                    return true;
                }
                pageReady = false;
                mainFrameRetries = 0;
                recreateWebView();
                return true;
            }

            @Override
            public WebResourceResponse shouldInterceptRequest(WebView wv, WebResourceRequest request) {
                return interceptWebRequest(request);
            }

            @Override
            public boolean shouldOverrideUrlLoading(WebView wv, WebResourceRequest request) {
                Uri uri = request.getUrl();
                if (uri != null && APP_HOST.equalsIgnoreCase(uri.getHost())) {
                    return false;
                }
                if (uri != null) {
                    gpuiOpenUrl(uri.toString());
                }
                return true;
            }

            @Override
            public void onPageStarted(WebView wv, String url, android.graphics.Bitmap favicon) {
                if (wv != webView) {
                    return;
                }
                mainFrameErrorThisLoad = false;
                if (readyPageToken < latestPageToken.get()) {
                    pageReady = false;
                }
            }

            @Override
            public void onPageFinished(WebView wv, String url) {
                if (wv != webView) {
                    return;
                }
                if (!mainFrameErrorThisLoad && !showingFallbackError && url != null && url.startsWith(APP_ORIGIN)) {
                    mainFrameRetries = 0;
                    int latest = latestPageToken.get();
                    if (latest > 0 && readyPageToken < latest) {
                        readyPageToken = latest;
                    }
                    pageReady = true;
                    flushPending();
                }
            }
        });

        ViewCompat.setOnApplyWindowInsetsListener(view, (v, insets) -> {
            Insets safe = insets.getInsets(WindowInsetsCompat.Type.systemBars() | WindowInsetsCompat.Type.displayCutout());
            Insets ime = insets.getInsets(WindowInsetsCompat.Type.ime());
            nativeInsets(safe.top, safe.right, safe.bottom, safe.left, ime.top, ime.right, ime.bottom, ime.left);
            applyCssInsets(safe, ime);
            return WindowInsetsCompat.CONSUMED;
        });

        webView = view;
        setContentView(view);
        ViewCompat.requestApplyInsets(view);
        loadAppPage();
    }

    private void loadAppPage() {
        if (webView == null) {
            return;
        }
        showingFallbackError = false;
        mainFrameErrorThisLoad = false;
        pageReady = false;
        webView.loadUrl(APP_ORIGIN + "/index.html" + (isSheet() ? "#share" : ""));
    }

    /**
     * A WebView can come back from the background black: the renderer was
     * frozen or its GPU surface lost, or the page's script died, while the
     * activity itself is fine. Ask the page for a sign of life; no answer in
     * time recreates the WebView, an empty app root reloads the page. Before,
     * only clearing the cache or force-closing the app helped.
     */
    private void checkPageAlive() {
        if (webView == null || showingFallbackError || isFinishing() || isDestroyed() || activityPaused) {
            return;
        }
        final WebView wv = webView;
        final int seq = ++healthSeq;
        final boolean[] answered = {false};
        try {
            wv.evaluateJavascript(
                    "(function(){var a=document.getElementById('app');"
                            + "return (a&&a.childElementCount>0?'ok':'empty')+'|'+document.readyState;})()",
                    value -> {
                        answered[0] = true;
                        if (wv != webView || seq != healthSeq) {
                            return;
                        }
                        String v = value == null ? "" : value;
                        if (v.contains("empty") && v.contains("complete")) {
                            Log.w("glyph-web", "App root empty after resume; reloading page");
                            mainFrameRetries = 0;
                            loadAppPage();
                        } else {
                            // Alive: repaint once, in case the surface came back blank.
                            wv.invalidate();
                        }
                    });
        } catch (Throwable t) {
            Log.w("glyph-web", "Health check failed", t);
            recreateWebView();
            return;
        }
        themeHandler.postDelayed(() -> {
            if (!answered[0] && wv == webView && seq == healthSeq && !isFinishing() && !isDestroyed() && !activityPaused) {
                Log.w("glyph-web", "WebView did not answer after resume; recreating");
                mainFrameRetries = 0;
                recreateWebView();
            }
        }, HEALTH_TIMEOUT_MS);
    }

    private void recreateWebView() {
        if (isFinishing() || isDestroyed()) {
            return;
        }
        WebView old = webView;
        webView = null;
        pageReady = false;
        pendingInvokes.clear();
        pendingEvents.clear();
        pendingLatest.clear();
        pendingLibrary.clear();
        if (old != null) {
            try {
                ViewParent parent = old.getParent();
                if (parent instanceof ViewGroup) {
                    ((ViewGroup) parent).removeView(old);
                }
                old.destroy();
            } catch (Throwable t) {
                Log.w("glyph-web", "Failed destroying dead WebView", t);
            }
        }
        setupWebView();
    }

    private void handleMainFrameFailure(String reason) {
        mainFrameErrorThisLoad = true;
        pageReady = false;
        if (isFinishing() || isDestroyed()) {
            return;
        }
        if (mainFrameRetries < MAX_MAIN_FRAME_RETRIES) {
            int attempt = ++mainFrameRetries;
            themeHandler.postDelayed(() -> {
                if (webView != null && !isFinishing() && !isDestroyed()) {
                    loadAppPage();
                }
            }, 250L * attempt);
        } else {
            showWebFallbackError(reason);
        }
    }

    private void showWebFallbackError(String reason) {
        if (webView == null || isFinishing() || isDestroyed()) {
            return;
        }
        showingFallbackError = true;
        pageReady = false;
        String safe = escapeHtml(reason == null || reason.isEmpty() ? "Unable to load interface." : reason);
        String html = "<!doctype html><html><head><meta charset='utf-8'>"
                + "<meta name='viewport' content='width=device-width,initial-scale=1'>"
                + "<style>html,body{margin:0;height:100%;background:#030303;color:#f5f5f5;"
                + "font-family:system-ui,-apple-system,sans-serif;display:flex;align-items:center;"
                + "justify-content:center;padding:24px;box-sizing:border-box}"
                + ".card{max-width:360px;width:100%;background:#111;border:1px solid #262626;"
                + "border-radius:16px;padding:20px;text-align:center}"
                + "h1{margin:0 0 8px;font-size:17px;font-weight:600}"
                + "p{margin:0 0 16px;font-size:13px;color:#a3a3a3;word-break:break-word}"
                + "a{display:inline-block;padding:10px 20px;border-radius:999px;background:#fff;"
                + "color:#000;text-decoration:none;font-size:14px;font-weight:600}</style></head>"
                + "<body><div class='card'><h1>Glyph</h1><p>" + safe + "</p>"
                + "<a href='" + APP_ORIGIN + "/index.html'>Retry</a></div></body></html>";
        webView.loadDataWithBaseURL(APP_ORIGIN + "/", html, "text/html", "utf-8", null);
    }

    private static String escapeHtml(String s) {
        StringBuilder out = new StringBuilder(s.length());
        for (int i = 0; i < s.length(); i++) {
            char c = s.charAt(i);
            switch (c) {
                case '&': out.append("&amp;"); break;
                case '<': out.append("&lt;"); break;
                case '>': out.append("&gt;"); break;
                case '"': out.append("&quot;"); break;
                case '\'': out.append("&#39;"); break;
                default: out.append(c); break;
            }
        }
        return out.toString();
    }

    private void applyCssInsets(Insets safe, Insets ime) {
        float density = getResources().getDisplayMetrics().density;
        if (density <= 0f) {
            density = 1f;
        }
        String js = String.format(Locale.US,
                "(function(r){r.setProperty('--safe-top','%.2fpx');r.setProperty('--safe-right','%.2fpx');"
                        + "r.setProperty('--safe-bottom','%.2fpx');r.setProperty('--safe-left','%.2fpx');"
                        + "r.setProperty('--ime-bottom','%.2fpx');})(document.documentElement.style);",
                safe.top / density, safe.right / density, safe.bottom / density, safe.left / density,
                ime.bottom / density);
        lastInsetsJs = js;
        if (pageReady && webView != null) {
            webView.evaluateJavascript(js, null);
        }
    }

    private final class Bridge {
        @JavascriptInterface
        public void ready(int token) {
            runOnUiThread(() -> markBridgeReady(token));
        }

        @JavascriptInterface
        public void invoke(long id, String cmd, String argsJson) {
            nativeInvoke(id, cmd == null ? "" : cmd, argsJson == null ? "{}" : argsJson);
        }

        /** Share sheet: what was shared (JSON), or null. */
        @JavascriptInterface
        public String sharePayload() {
            return sharePayload;
        }

        /** Share sheet: done or closed; Android says `message` once it is gone. */
        @JavascriptInterface
        public void closeSheet(String message) {
            runOnUiThread(() -> closeSheetNow(message));
        }

        /** Share sheet: only the app can do this (a jam, a search): open it with what was shared. */
        @JavascriptInterface
        public void openInApp() {
            runOnUiThread(MainActivity.this::openShareInApp);
        }
    }

    private void markBridgeReady(int token) {
        if (token <= 0 || token < latestPageToken.get() || webView == null) {
            return;
        }
        showingFallbackError = false;
        mainFrameRetries = 0;
        readyPageToken = token;
        pageReady = true;
        flushPending();
    }

    private void flushPending() {
        if (webView == null) {
            return;
        }
        if (lastInsetsJs != null) {
            webView.evaluateJavascript(lastInsetsJs, null);
        }
        // In the background the page is frozen: keep everything until onResume.
        if (activityPaused) {
            return;
        }
        if (!pendingInvokes.isEmpty()) {
            List<String> copy = new ArrayList<>(pendingInvokes);
            pendingInvokes.clear();
            for (String js : copy) {
                webView.evaluateJavascript(js, null);
            }
        }
        if (!pendingEvents.isEmpty()) {
            List<String> copy = new ArrayList<>(pendingEvents);
            pendingEvents.clear();
            for (String js : copy) {
                webView.evaluateJavascript(js, null);
            }
        }
        if (!pendingLatest.isEmpty()) {
            List<String> copy = new ArrayList<>(pendingLatest.values());
            pendingLatest.clear();
            for (String js : copy) {
                webView.evaluateJavascript(js, null);
            }
        }
        if (!pendingLibrary.isEmpty()) {
            List<String> copy = new ArrayList<>(pendingLibrary);
            pendingLibrary.clear();
            for (String js : copy) {
                webView.evaluateJavascript(js, null);
            }
        }
    }

    private void evalInvokeJs(String js) {
        runOnUiThread(() -> {
            if (webView != null && pageReady && !activityPaused) {
                webView.evaluateJavascript(js, null);
            } else if (!isFinishing() && !isDestroyed()) {
                pendingInvokes.add(js);
            }
        });
    }

    private void evalEventJs(String name, String js, boolean patch) {
        runOnUiThread(() -> {
            if (webView != null && pageReady && !activityPaused) {
                webView.evaluateJavascript(js, null);
            } else if (isFinishing() || isDestroyed()) {
                return;
            } else if ("glyph://library".equals(name)) {
                // A whole library makes the waiting ones useless; patches
                // keep their order. Too many: the next whole one is asked for.
                if (!patch) {
                    pendingLibrary.clear();
                }
                if (pendingLibrary.size() < MAX_PENDING_LIBRARY) {
                    pendingLibrary.add(js);
                } else {
                    pendingLibrary.clear();
                    libraryResendNeeded = true;
                }
            } else if (LATEST_ONLY.contains(name)) {
                pendingLatest.remove(name);
                pendingLatest.put(name, js);
            } else {
                if (pendingEvents.size() >= MAX_PENDING_EVENTS) {
                    pendingEvents.remove(0);
                }
                pendingEvents.add(js);
            }
        });
    }

    /** A library event with only the changed parts (`{"__patch":{..}}`). */
    private static boolean isLibraryPatch(String payloadJson) {
        return payloadJson != null && payloadJson.startsWith("{\"__patch\":");
    }

    /** Library patches were dropped while away: Rust sends one whole library on return. */
    private boolean libraryResendNeeded;

    private static String toSafeJsLiteral(String json) {
        if (json == null || json.isEmpty()) {
            return "null";
        }
        return json.replace("\u2028", "\\u2028").replace("\u2029", "\\u2029");
    }

    private WebResourceResponse interceptWebRequest(WebResourceRequest request) {
        Uri uri = request.getUrl();
        if (uri == null) {
            return null;
        }
        String host = uri.getHost();
        if ("asset.localhost".equalsIgnoreCase(host)) {
            String rawPath = uri.getPath();
            if (rawPath == null || rawPath.isEmpty()) {
                return textResponse(404, "Not Found");
            }
            return serveLocalAssetFile(Uri.decode(rawPath));
        }
        if (!APP_HOST.equalsIgnoreCase(host)) {
            return null;
        }
        String path = uri.getPath();
        if (path == null || path.isEmpty()) {
            path = "/index.html";
        }
        if (ASSET_PREFIX.equals(path)) {
            String filePath = uri.getQueryParameter("path");
            if (filePath == null || filePath.isEmpty()) {
                return textResponse(400, "Missing path");
            }
            return serveLocalAssetFile(filePath);
        }
        return serveUiAsset(path, request.isForMainFrame());
    }

    private WebResourceResponse serveLocalAssetFile(String filePath) {
        try {
            File root = getFilesDir().getCanonicalFile();
            File target = new File(filePath).getCanonicalFile();
            String rootPath = root.getPath();
            String targetPath = target.getPath();
            if (!targetPath.equals(rootPath) && !targetPath.startsWith(rootPath + File.separator)) {
                return textResponse(403, "Forbidden");
            }
            if (!target.isFile()) {
                return textResponse(404, "Not Found");
            }
            return new WebResourceResponse(mimeTypeFor(targetPath), null, new FileInputStream(target));
        } catch (IOException e) {
            return textResponse(404, "Not Found");
        }
    }

    private WebResourceResponse serveUiAsset(String rawPath, boolean mainFrame) {
        String rel = rawPath.startsWith("/") ? rawPath.substring(1) : rawPath;
        if (rel.isEmpty() || rel.endsWith("/")) {
            rel = rel + "index.html";
        }
        if (rel.contains("..")) {
            return textResponse(403, "Forbidden");
        }
        try {
            InputStream stream = getAssets().open("ui/" + rel);
            if ("index.html".equals(rel)) {
                return injectBridge(stream, mainFrame);
            }
            return new WebResourceResponse(mimeTypeFor(rel), "utf-8", stream);
        } catch (IOException notFound) {
            // Client-side route without file extension falls back to index.html.
            int slash = rel.lastIndexOf('/');
            String tail = slash >= 0 ? rel.substring(slash + 1) : rel;
            if (mainFrame && !tail.contains(".")) {
                try {
                    return injectBridge(getAssets().open("ui/index.html"), true);
                } catch (IOException ignored) {
                }
            }
            return textResponse(404, "Not Found");
        }
    }

    private WebResourceResponse injectBridge(InputStream stream, boolean mainFrame) throws IOException {
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        byte[] buf = new byte[8192];
        int n;
        while ((n = stream.read(buf)) > 0) {
            out.write(buf, 0, n);
        }
        stream.close();
        int token = mainFrame ? nextPageToken.incrementAndGet() : 0;
        if (token > 0) {
            latestPageToken.set(token);
        }
        String html = out.toString(StandardCharsets.UTF_8.name());
        String script = "<script>" + tauriShimScript(token) + "</script>";
        int idx = html.indexOf("<head>");
        String patched = idx >= 0
                ? html.substring(0, idx + 6) + script + html.substring(idx + 6)
                : script + html;
        Map<String, String> headers = new HashMap<>();
        headers.put("Cache-Control", "no-store, no-cache, must-revalidate");
        return new WebResourceResponse("text/html", "utf-8", 200, "OK", headers,
                new ByteArrayInputStream(patched.getBytes(StandardCharsets.UTF_8)));
    }

    private static String tauriShimScript(int pageToken) {
        long initialReq = pageToken > 0 ? (long) pageToken * 1000000L + 1L : 1L;
        return "(function(){"
                + "if(window.__TAURI_INTERNALS__){"
                + (pageToken > 0 ? "try{window.__GLYPH_ANDROID__.ready(" + pageToken + ");}catch(_e){}" : "")
                + "return;}"
                + "var nextCb=1,nextReq=" + initialReq + ",cbs=new Map(),pending=new Map(),listeners=new Map();"
                + "window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener:function(){}};"
                + "function runCb(id,arg){var fn=cbs.get(id);if(fn){try{fn(arg);}catch(e){console.error(e);}}}"
                + "function winNoop(cmd){"
                + "var dpr=window.devicePixelRatio||1;"
                + "if(cmd==='plugin:window|is_maximized'||cmd==='plugin:window|is_fullscreen'||cmd==='plugin:window|is_minimized')return false;"
                + "if(cmd==='plugin:window|is_focused'||cmd==='plugin:window|is_visible')return true;"
                + "if(cmd==='plugin:window|scale_factor')return dpr;"
                + "if(cmd==='plugin:window|inner_size'||cmd==='plugin:window|outer_size')return{width:Math.round(window.innerWidth*dpr),height:Math.round(window.innerHeight*dpr)};"
                + "return null;}"
                + "window.__glyphResolve=function(id,ok,payload){"
                + "var p=pending.get(id);if(!p)return;pending.delete(id);"
                + "if(ok)p.resolve(payload);else p.reject(typeof payload==='string'?payload:((payload&&payload.message)||String(payload)));};"
                + "window.__glyphEmit=function(name,payload){"
                + "var map=listeners.get(name);if(!map)return;"
                + "map.forEach(function(cbId,lid){runCb(cbId,{event:name,id:lid,payload:payload});});};"
                + "window.__TAURI_INTERNALS__={"
                + "metadata:{currentWindow:{label:'main'},currentWebview:{windowLabel:'main',label:'main'}},"
                + "callbacks:cbs,"
                + "transformCallback:function(cb,once){var id=nextCb++;cbs.set(id,function(d){if(once)cbs.delete(id);return cb&&cb(d);});return id;},"
                + "unregisterCallback:function(id){cbs.delete(id);},"
                + "convertFileSrc:function(path){if(!path)return '';if(/^(https?:|data:|blob:)/.test(path))return path;return 'https://glyph.localhost/__asset__?path='+encodeURIComponent(path);},"
                + "invoke:function(cmd,args){"
                + "if(cmd==='plugin:event|listen'){"
                + "var ev=args&&args.event,cbId=args&&args.handler;"
                + "var lid=nextCb++;if(!listeners.has(ev))listeners.set(ev,new Map());"
                + "listeners.get(ev).set(lid,cbId);return Promise.resolve(lid);}"
                + "if(cmd==='plugin:event|unlisten'){"
                + "var m=listeners.get(args&&args.event);if(m)m.delete(args&&args.eventId);return Promise.resolve(null);}"
                + "if(cmd==='plugin:event|emit'||cmd==='plugin:event|emit_to'){"
                + "window.__glyphEmit(args&&args.event,args&&args.payload);return Promise.resolve(null);}"
                + "if(cmd&&cmd.indexOf('plugin:')===0){return Promise.resolve(winNoop(cmd));}"
                + "return new Promise(function(resolve,reject){"
                + "var id=nextReq++;pending.set(id,{resolve:resolve,reject:reject});"
                + "try{window.__GLYPH_ANDROID__.invoke(id,cmd,JSON.stringify(args===undefined?{}:args));}"
                + "catch(e){pending.delete(id);reject(e);}"
                + "});}"
                + "};"
                + (pageToken > 0 ? "try{window.__GLYPH_ANDROID__.ready(" + pageToken + ");}catch(_e){}" : "")
                + "})();";
    }

    private static WebResourceResponse textResponse(int code, String msg) {
        return new WebResourceResponse("text/plain", "utf-8", code, msg, null,
                new ByteArrayInputStream(msg.getBytes(StandardCharsets.UTF_8)));
    }

    private static String mimeTypeFor(String path) {
        String lower = path.toLowerCase(Locale.US);
        if (lower.endsWith(".html")) return "text/html";
        if (lower.endsWith(".js") || lower.endsWith(".mjs")) return "application/javascript";
        if (lower.endsWith(".css")) return "text/css";
        if (lower.endsWith(".json")) return "application/json";
        if (lower.endsWith(".svg")) return "image/svg+xml";
        if (lower.endsWith(".png")) return "image/png";
        if (lower.endsWith(".jpg") || lower.endsWith(".jpeg")) return "image/jpeg";
        if (lower.endsWith(".webp")) return "image/webp";
        if (lower.endsWith(".gif")) return "image/gif";
        if (lower.endsWith(".woff2")) return "font/woff2";
        if (lower.endsWith(".woff")) return "font/woff";
        if (lower.endsWith(".ttf")) return "font/ttf";
        if (lower.endsWith(".otf")) return "font/otf";
        if (lower.endsWith(".wasm")) return "application/wasm";
        if (lower.endsWith(".mp3")) return "audio/mpeg";
        if (lower.endsWith(".flac")) return "audio/flac";
        if (lower.endsWith(".ogg") || lower.endsWith(".opus")) return "audio/ogg";
        if (lower.endsWith(".m4a")) return "audio/mp4";
        return "application/octet-stream";
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
        if (isSheet()) {
            takeShare(intent);
            return;
        }
        takeLink(intent);
    }

    // ------------------------------------------------------------ share sheet

    /** What was shared to the sheet: {"text":..}, {"files":[..]} or {"loading":"files"}. */
    private volatile String sharePayload;
    private Intent shareIntent;
    private boolean sheetClosing;

    /** Share sheet: reads the intent and hands it to the page. */
    private void takeShare(Intent intent) {
        if (intent == null) {
            closeSheetNow("");
            return;
        }
        shareIntent = intent;
        List<Uri> audio = audioUris(intent);
        try {
            if (!audio.isEmpty()) {
                sharePayload = new JSONObject().put("loading", "files").toString();
                pushShare();
                new Thread(() -> {
                    org.json.JSONArray paths = new org.json.JSONArray();
                    for (String path : copyAudioFiles(audio)) {
                        paths.put(path);
                    }
                    String json;
                    try {
                        // None could be read: the sheet says so instead of waiting.
                        json = paths.length() == 0
                                ? new JSONObject().put("error", "files").toString()
                                : new JSONObject().put("files", paths).toString();
                    } catch (org.json.JSONException e) {
                        return;
                    }
                    runOnUiThread(() -> {
                        sharePayload = json;
                        pushShare();
                    });
                }, "glyph-share-files").start();
                return;
            }
            CharSequence text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT);
            if (text == null || text.length() == 0) {
                closeSheetNow("");
                return;
            }
            sharePayload = new JSONObject().put("text", text.toString()).toString();
        } catch (org.json.JSONException e) {
            closeSheetNow("");
            return;
        }
        pushShare();
    }

    /** A page that already runs gets a new share right away. */
    private void pushShare() {
        if (webView != null && pageReady && sharePayload != null) {
            webView.evaluateJavascript("window.__glyphShare&&window.__glyphShare("
                    + toSafeJsLiteral(sharePayload) + ");", null);
        }
    }

    private void closeSheetNow(String message) {
        if (sheetClosing || isFinishing()) {
            return;
        }
        sheetClosing = true;
        if (message != null && !message.isEmpty()) {
            android.widget.Toast.makeText(getApplicationContext(), message, android.widget.Toast.LENGTH_SHORT).show();
        }
        finish();
        applyTransition(false, 0, android.R.anim.fade_out);
    }

    @SuppressWarnings("deprecation")
    private void applyTransition(boolean open, int enterAnim, int exitAnim) {
        if (Build.VERSION.SDK_INT >= 34) {
            overrideActivityTransition(open ? OVERRIDE_TRANSITION_OPEN : OVERRIDE_TRANSITION_CLOSE, enterAnim, exitAnim);
        } else {
            overridePendingTransition(enterAnim, exitAnim);
        }
    }

    /** Hands what was shared to the app itself and ends the sheet. */
    private void openShareInApp() {
        Intent i = new Intent(this, MainActivity.class);
        Intent from = shareIntent;
        if (from != null) {
            i.setAction(from.getAction());
            if (from.getData() != null || from.getType() != null) {
                i.setDataAndType(from.getData(), from.getType());
            }
            if (from.getExtras() != null) {
                i.putExtras(from.getExtras());
            }
            i.setClipData(from.getClipData());
        }
        i.addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_GRANT_READ_URI_PERMISSION);
        try {
            startActivity(i);
        } catch (Exception e) {
            Log.w("glyph", "opening the app failed", e);
        }
        closeSheetNow("");
    }

    /** Back from the permission screen: go on with the update once allowed. */
    @Override
    protected void onResume() {
        activityPaused = false;
        super.onResume();
        if (webView != null) {
            webView.onResume();
            // Hand over what waited while the app was in the background.
            if (pageReady) {
                flushPending();
            }
        }
        nativeResumed();
        if (libraryResendNeeded) {
            libraryResendNeeded = false;
            nativeLibraryResend();
        }
        if (checkOnResume) {
            checkOnResume = false;
            // Give the WebView a moment to wake its renderer before asking.
            themeHandler.postDelayed(this::checkPageAlive, 600L);
        }
        if (connectivity != null) {
            reportConnectivity();
        }
        if (sensorManager != null && accelerometer != null) {
            sensorManager.registerListener(shakeListener, accelerometer, SensorManager.SENSOR_DELAY_UI);
        }
        if (pendingApk != null && getPackageManager().canRequestPackageInstalls()) {
            String path = pendingApk;
            pendingApk = null;
            installApk(path);
        }
    }

    @Override
    protected void onPause() {
        if (sensorManager != null) {
            sensorManager.unregisterListener(shakeListener);
        }
        shakePeakCount = 0;
        super.onPause();
    }

    @Override
    protected void onStop() {
        activityPaused = true;
        nativePaused();
        if (isSheet() && !isChangingConfigurations()) {
            // The sheet belongs to the moment of sharing: once it is out of
            // sight it is done.
            closeSheetNow("");
        }
        checkOnResume = true;
        if (webView != null) {
            webView.onPause();
        }
        super.onStop();
        if (pendingAlias != null) {
            themeHandler.removeCallbacks(applyAliasRunnable);
            applyLauncherAliasNow(pendingAlias);
        }
    }

    @Override
    protected void onDestroy() {
        themeHandler.removeCallbacksAndMessages(null);
        if (connectivity != null && networkCallback != null) {
            try {
                connectivity.unregisterNetworkCallback(networkCallback);
            } catch (Exception ignored) {
            }
            networkCallback = null;
        }
        if (webView != null) {
            try {
                ViewParent parent = webView.getParent();
                if (parent instanceof ViewGroup) {
                    ((ViewGroup) parent).removeView(webView);
                }
                webView.destroy();
            } catch (Throwable ignored) {
            }
            webView = null;
        }
        nativeDetach(this);
        super.onDestroy();
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
        if (intent == null) {
            return;
        }
        List<Uri> audio = audioUris(intent);
        if (!audio.isEmpty()) {
            takeAudioFiles(audio);
            return;
        }
        String url;
        if (Intent.ACTION_SEND.equals(intent.getAction())) {
            // Shared from another app (a Spotify / YouTube link, a song name):
            // Rust treats it like text pasted into the search bar.
            CharSequence text = intent.getCharSequenceExtra(Intent.EXTRA_TEXT);
            if (text == null || text.length() == 0) {
                return;
            }
            url = "glyph-shared:" + text;
        } else if (Intent.ACTION_VIEW.equals(intent.getAction()) && intent.getData() != null) {
            url = intent.getData().toString();
        } else {
            return;
        }
        synchronized (this) {
            openUrl = url;
        }
        nativeUrlWaiting();
    }

    /** At most this many files from one "Open with" / share. */
    private static final int MAX_OPENED_FILES = 200;
    /** Files larger than this are not copied (a whole video, by mistake). */
    private static final long MAX_OPENED_BYTES = 1L << 30;

    /**
     * Music files this intent hands over: "Open with" (VIEW of an audio
     * file) or "Share" of one or more audio files (SEND / SEND_MULTIPLE).
     */
    @SuppressWarnings("deprecation")
    private List<Uri> audioUris(Intent intent) {
        List<Uri> out = new ArrayList<>();
        String action = intent.getAction();
        String type = intent.getType();
        boolean audioType = type != null && type.startsWith("audio/");
        if (Intent.ACTION_VIEW.equals(action)) {
            Uri data = intent.getData();
            if (data != null && audioType && ("content".equals(data.getScheme()) || "file".equals(data.getScheme()))) {
                out.add(data);
            }
        } else if (Intent.ACTION_SEND.equals(action) && audioType) {
            Uri u = intent.getParcelableExtra(Intent.EXTRA_STREAM);
            if (u != null) {
                out.add(u);
            }
        } else if (Intent.ACTION_SEND_MULTIPLE.equals(action) && audioType) {
            ArrayList<Uri> list = intent.getParcelableArrayListExtra(Intent.EXTRA_STREAM);
            if (list != null) {
                for (Uri u : list) {
                    if (u != null && out.size() < MAX_OPENED_FILES) {
                        out.add(u);
                    }
                }
            }
        }
        return out;
    }

    /**
     * Copies opened music files into the app's storage (`files/opened`), off
     * the main thread, and hands their paths to Rust ("glyph-file:", one per
     * line). The copy stays, so a song liked or added from the sheet keeps
     * playing from it; the same file shared again is not copied twice.
     */
    private void takeAudioFiles(List<Uri> uris) {
        new Thread(() -> {
            StringBuilder paths = new StringBuilder();
            for (String path : copyAudioFiles(uris)) {
                paths.append(path).append('\n');
            }
            if (paths.length() == 0) {
                return;
            }
            synchronized (MainActivity.this) {
                openUrl = "glyph-file:" + paths;
            }
            runOnUiThread(this::nativeUrlWaiting);
        }, "glyph-open-files").start();
    }

    /** Copies the files into `files/opened` (off the main thread); their paths. */
    private List<String> copyAudioFiles(List<Uri> uris) {
        List<String> out = new ArrayList<>();
        File dir = new File(getFilesDir(), "opened");
        if (!dir.isDirectory() && !dir.mkdirs()) {
            return out;
        }
        for (Uri uri : uris) {
            File f = copyOpened(uri, dir);
            if (f != null) {
                out.add(f.getAbsolutePath());
            }
        }
        return out;
    }

    private File copyOpened(Uri uri, File dir) {
        String name = null;
        long size = -1;
        try (android.database.Cursor c = getContentResolver().query(uri,
                new String[] {android.provider.OpenableColumns.DISPLAY_NAME, android.provider.OpenableColumns.SIZE},
                null, null, null)) {
            if (c != null && c.moveToFirst()) {
                if (!c.isNull(0)) {
                    name = c.getString(0);
                }
                if (!c.isNull(1)) {
                    size = c.getLong(1);
                }
            }
        } catch (Exception e) {
            Log.i("glyph", "opened file: no name", e);
        }
        if (name == null) {
            name = uri.getLastPathSegment();
        }
        name = openedName(name, getContentResolver().getType(uri));
        if (name == null || size > MAX_OPENED_BYTES) {
            return null;
        }
        File dest = new File(dir, name);
        if (dest.isFile() && size >= 0 && dest.length() == size) {
            return dest;
        }
        File tmp = new File(dir, name + ".part");
        try (InputStream in = getContentResolver().openInputStream(uri);
             OutputStream out = new java.io.FileOutputStream(tmp)) {
            if (in == null) {
                return null;
            }
            byte[] buf = new byte[1 << 16];
            long total = 0;
            int n;
            while ((n = in.read(buf)) > 0) {
                total += n;
                if (total > MAX_OPENED_BYTES) {
                    throw new IOException("file too large");
                }
                out.write(buf, 0, n);
            }
        } catch (Exception e) {
            Log.w("glyph", "opened file not copied", e);
            //noinspection ResultOfMethodCallIgnored
            tmp.delete();
            return null;
        }
        if (dest.exists() && !dest.delete()) {
            //noinspection ResultOfMethodCallIgnored
            tmp.delete();
            return null;
        }
        return tmp.renameTo(dest) ? dest : null;
    }

    /** Audio file endings Glyph plays (glyph_core::import::local::AUDIO_EXTS). */
    private static final Set<String> AUDIO_EXTS = new HashSet<>(Arrays.asList(
            "mp3", "flac", "ogg", "oga", "m4a", "mp4", "aac", "wav", "mka"));

    /**
     * A safe file name for an opened file: no folders, and an ending Glyph
     * plays (taken from the type when the name has none). Null: not audio.
     */
    private static String openedName(String raw, String mime) {
        String name = raw == null ? "" : raw;
        int slash = Math.max(name.lastIndexOf('/'), name.lastIndexOf('\\'));
        name = name.substring(slash + 1).replaceAll("[^\\p{L}\\p{N} ._()\\-]", "_").trim();
        if (name.startsWith(".")) {
            name = "_" + name;
        }
        if (name.length() > 120) {
            name = name.substring(name.length() - 120);
        }
        int dot = name.lastIndexOf('.');
        String ext = dot >= 0 ? name.substring(dot + 1).toLowerCase(java.util.Locale.ROOT) : "";
        if (AUDIO_EXTS.contains(ext)) {
            return name;
        }
        String byType = extForType(mime);
        if (byType == null) {
            return null;
        }
        return (name.isEmpty() ? "song" : name) + "." + byType;
    }

    private static String extForType(String mime) {
        if (mime == null) {
            return null;
        }
        switch (mime.toLowerCase(java.util.Locale.ROOT)) {
            case "audio/mpeg":
            case "audio/mp3":
                return "mp3";
            case "audio/flac":
            case "audio/x-flac":
                return "flac";
            case "audio/ogg":
            case "audio/vorbis":
            case "application/ogg":
                return "ogg";
            case "audio/mp4":
            case "audio/x-m4a":
            case "audio/m4a":
                return "m4a";
            case "audio/aac":
            case "audio/aacp":
                return "aac";
            case "audio/wav":
            case "audio/x-wav":
            case "audio/wave":
                return "wav";
            case "audio/x-matroska":
                return "mka";
            default:
                return null;
        }
    }

    /**
     * While a network device plays, the volume keys set its volume instead
     * of the phone's (in the background the remote media session does it).
     */
    @Override
    public boolean dispatchKeyEvent(KeyEvent event) {
        int code = event.getKeyCode();
        if ((code == KeyEvent.KEYCODE_VOLUME_UP || code == KeyEvent.KEYCODE_VOLUME_DOWN)
                && PlaybackService.remoteVolumeActive()) {
            if (event.getAction() == KeyEvent.ACTION_DOWN) {
                PlaybackService.volumeKey(code == KeyEvent.KEYCODE_VOLUME_UP ? 1 : -1);
            }
            return true;
        }
        return super.dispatchKeyEvent(event);
    }

    // ------------------------------------------------------------ called from Rust

    public void gpuiCompleteInvoke(long id, boolean ok, String payloadJson) {
        evalInvokeJs("window.__glyphResolve(" + id + "," + ok + "," + toSafeJsLiteral(payloadJson) + ");");
    }

    public void gpuiEmitEvent(String name, String payloadJson) {
        String ev = name == null ? "" : name;
        if (activityPaused && "glyph://spectrum".equals(ev)) {
            return;
        }
        evalEventJs(ev, "window.__glyphEmit(" + JSONObject.quote(ev)
                + "," + toSafeJsLiteral(payloadJson) + ");", isLibraryPatch(payloadJson));
    }

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

    /**
     * Opens the system picker for one file (playlist picture, profile
     * picture, playlist import). The chosen file is copied into the cache,
     * because Rust reads a path; the answer goes to nativeFilePicked (null
     * when cancelled).
     */
    public void gpuiPickFile(String mimeTypes) {
        runOnUiThread(() -> {
            String[] mimes = mimeTypes == null || mimeTypes.isEmpty() ? new String[] {"*/*"} : mimeTypes.split(",");
            Intent pick = new Intent(Intent.ACTION_GET_CONTENT);
            pick.addCategory(Intent.CATEGORY_OPENABLE);
            pick.setType(mimes.length == 1 ? mimes[0] : "*/*");
            if (mimes.length > 1) {
                pick.putExtra(Intent.EXTRA_MIME_TYPES, mimes);
            }
            try {
                filePickLauncher.launch(Intent.createChooser(pick, null));
            } catch (Exception e) {
                Log.w("glyph", "file picker unavailable", e);
                nativeFilePicked(null);
            }
        });
    }

    private final ActivityResultLauncher<Intent> filePickLauncher =
            registerForActivityResult(new ActivityResultContracts.StartActivityForResult(), result -> {
                Uri uri = result.getResultCode() == RESULT_OK && result.getData() != null ? result.getData().getData() : null;
                if (uri == null) {
                    nativeFilePicked(null);
                    return;
                }
                new Thread(() -> nativeFilePicked(copyPicked(uri)), "glyph-pick").start();
            });

    /** Copies a picked document into the cache, keeping its name (its extension tells the type). */
    private String copyPicked(Uri uri) {
        String name = null;
        try (android.database.Cursor c = getContentResolver().query(uri,
                new String[] {android.provider.OpenableColumns.DISPLAY_NAME}, null, null, null)) {
            if (c != null && c.moveToFirst()) {
                name = c.getString(0);
            }
        } catch (Exception ignored) {
        }
        if (name == null || name.isEmpty()) {
            String ext = android.webkit.MimeTypeMap.getSingleton().getExtensionFromMimeType(getContentResolver().getType(uri));
            name = "picked" + (ext == null ? "" : "." + ext);
        }
        name = name.replaceAll("[\\\\/:*?\"<>|]", "_");
        File dir = new File(getCacheDir(), "picked");
        File[] old = dir.listFiles();
        if (old != null) {
            for (File f : old) {
                f.delete();
            }
        }
        dir.mkdirs();
        File out = new File(dir, name);
        try (InputStream in = getContentResolver().openInputStream(uri);
             OutputStream os = new java.io.FileOutputStream(out)) {
            if (in == null) {
                return null;
            }
            byte[] buf = new byte[64 * 1024];
            int n;
            while ((n = in.read(buf)) > 0) {
                os.write(buf, 0, n);
            }
            return out.getAbsolutePath();
        } catch (Exception e) {
            Log.w("glyph", "cannot read the picked file", e);
            return null;
        }
    }

    /** Opens the Android share sheet with a link or text (phone share menu, "More"). */
    public void gpuiShareText(String text, String title) {
        runOnUiThread(() -> {
            try {
                Intent send = new Intent(Intent.ACTION_SEND);
                send.setType("text/plain");
                send.putExtra(Intent.EXTRA_TEXT, text);
                if (title != null && !title.isEmpty()) {
                    send.putExtra(Intent.EXTRA_SUBJECT, title);
                }
                startActivity(Intent.createChooser(send, title == null || title.isEmpty() ? null : title));
            } catch (Exception ignored) {
            }
        });
    }

    /** Glyph's own notifications from Rust (`notify.rs`): one notice as JSON. */
    public void gpuiNotify(String json) {
        Notifications.post(getApplicationContext(), json);
    }

    public void gpuiSetClipboard(String text) {
        runOnUiThread(() -> {
            ClipboardManager cm = (ClipboardManager) getSystemService(Context.CLIPBOARD_SERVICE);
            if (cm != null) {
                cm.setPrimaryClip(ClipData.newPlainText("Glyph", text));
            }
        });
    }

    public String gpuiGetClipboard() {
        if (Looper.myLooper() == Looper.getMainLooper()) {
            return readClipboardNow();
        }
        AtomicReference<String> out = new AtomicReference<>();
        CountDownLatch latch = new CountDownLatch(1);
        runOnUiThread(() -> {
            try {
                out.set(readClipboardNow());
            } finally {
                latch.countDown();
            }
        });
        try {
            latch.await(2, TimeUnit.SECONDS);
        } catch (InterruptedException ignored) {
            Thread.currentThread().interrupt();
        }
        return out.get();
    }

    private String readClipboardNow() {
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
     * Liked Songs (the notification's add button shows a check); playing
     * also covers finding and buffering the next song. remoteVolume: a
     * network device plays at this volume (0..100) and the volume keys go
     * to it; -1 when the phone plays.
     */
    public void gpuiNowPlaying(String title, String artist, boolean playing, boolean buffering, long positionMs, long durationMs,
                               String coverPath, boolean liked, int remoteVolume) {
        runOnUiThread(() -> PlaybackService.update(this, title, artist, playing, buffering, positionMs, durationMs, coverPath, liked,
                remoteVolume));
    }

    /** Keeps the foreground service and wake/Wi-Fi locks alive while in a Jam even without active audio. */
    public void gpuiJamActive(boolean active) {
        runOnUiThread(() -> PlaybackService.setJamActive(this, active));
    }

    /** ANDROID_ID; Rust only sends a salted hash of it (glyph_core::hardware). */
    public String gpuiAndroidId() {
        return Settings.Secure.getString(getContentResolver(), Settings.Secure.ANDROID_ID);
    }

    /** The link the app was opened with, once (null when there is none). */
    public synchronized String gpuiTakeOpenUrl() {
        String url = openUrl;
        openUrl = null;
        return url;
    }

    @SuppressWarnings("deprecation")
    public float gpuiRefreshRate() {
        Display d = getWindowManager().getDefaultDisplay();
        return d == null ? 60f : d.getRefreshRate();
    }

    /** Updates Android Recents icon immediately and schedules launcher alias switch. */
    public void gpuiSetThemeIcon(String themeId, int accentRgb) {
        if (isSheet()) {
            return;
        }
        int rgb = accentRgb & 0x00ffffff;
        runOnUiThread(() -> {
            updateTaskDescription(rgb);
            scheduleLauncherAlias(aliasForTheme(themeId, rgb));
        });
    }

    @SuppressWarnings("deprecation")
    private void updateTaskDescription(int rgb) {
        if (lastAccentRgb == rgb) {
            return;
        }
        lastAccentRgb = rgb;
        Bitmap bmp = tintLauncherBitmap(rgb);
        if (bmp != null) {
            setTaskDescription(new ActivityManager.TaskDescription("Glyph", bmp));
        }
    }

    private Bitmap tintLauncherBitmap(int rgb) {
        Bitmap src = BitmapFactory.decodeResource(getResources(), R.mipmap.ic_launcher);
        if (src == null) {
            return null;
        }
        if (rgb == DEFAULT_ACCENT) {
            return src;
        }
        Bitmap out = src.copy(Bitmap.Config.ARGB_8888, true);
        if (out == null) {
            return src;
        }
        int w = out.getWidth();
        int h = out.getHeight();
        int[] px = new int[w * h];
        out.getPixels(px, 0, w, 0, 0, w, h);
        float tr = (rgb >> 16) & 0xff;
        float tg = (rgb >> 8) & 0xff;
        float tb = rgb & 0xff;
        final float mix = 0.72f;
        float sr = (mix * tr + (1.0f - mix) * 255.0f) / 255.0f;
        float sg = (mix * tg + (1.0f - mix) * 255.0f) / 255.0f;
        float sb = (mix * tb + (1.0f - mix) * 255.0f) / 255.0f;
        for (int i = 0; i < px.length; i++) {
            int p = px[i];
            int a = (p >>> 24) & 0xff;
            if (a == 0) {
                continue;
            }
            float r = (p >> 16) & 0xff;
            float g = (p >> 8) & 0xff;
            float b = p & 0xff;
            if (r <= g || b <= g) {
                continue;
            }
            float intensity = Math.min(r, Math.min((r - g) / 0.3125f, (b - g) / 0.256f));
            if (intensity <= 0.5f) {
                continue;
            }
            float white = Math.max(0.0f, r - intensity);
            int nr = Math.min(255, Math.max(0, Math.round(white + intensity * sr)));
            int ng = Math.min(255, Math.max(0, Math.round(white + intensity * sg)));
            int nb = Math.min(255, Math.max(0, Math.round(white + intensity * sb)));
            px[i] = (a << 24) | (nr << 16) | (ng << 8) | nb;
        }
        out.setPixels(px, 0, w, 0, 0, w, h);
        return out;
    }

    private static String aliasForTheme(String themeId, int rgb) {
        if (themeId != null) {
            for (int i = 0; i < LAUNCHER_THEMES.length; i++) {
                if (LAUNCHER_THEMES[i].equals(themeId)) {
                    return LAUNCHER_ALIASES[i];
                }
            }
        }
        int r = (rgb >> 16) & 0xff;
        int g = (rgb >> 8) & 0xff;
        int b = rgb & 0xff;
        int bestIdx = 0;
        int bestDist = Integer.MAX_VALUE;
        for (int i = 0; i < LAUNCHER_ACCENTS.length; i++) {
            int c = LAUNCHER_ACCENTS[i];
            int dr = r - ((c >> 16) & 0xff);
            int dg = g - ((c >> 8) & 0xff);
            int db = b - (c & 0xff);
            int d = dr * dr + dg * dg + db * db;
            if (d < bestDist) {
                bestDist = d;
                bestIdx = i;
            }
        }
        return LAUNCHER_ALIASES[bestIdx];
    }

    private void scheduleLauncherAlias(String alias) {
        pendingAlias = alias;
        themeHandler.removeCallbacks(applyAliasRunnable);
        themeHandler.postDelayed(applyAliasRunnable, 1000L);
    }

    private void applyLauncherAliasNow(String targetAlias) {
        pendingAlias = null;
        if (targetAlias == null) {
            return;
        }
        final Context appCtx = getApplicationContext();
        THEME_EXECUTOR.execute(() -> {
            try {
                PackageManager pm = appCtx.getPackageManager();
                String pkg = appCtx.getPackageName();
                if (!isAliasEnabled(pm, pkg, targetAlias, LAUNCHER_ALIASES[0].equals(targetAlias))) {
                    pm.setComponentEnabledSetting(
                            new ComponentName(pkg, targetAlias),
                            PackageManager.COMPONENT_ENABLED_STATE_ENABLED,
                            PackageManager.DONT_KILL_APP
                    );
                }
                for (int i = 0; i < LAUNCHER_ALIASES.length; i++) {
                    String cls = LAUNCHER_ALIASES[i];
                    if (cls.equals(targetAlias)) {
                        continue;
                    }
                    if (isAliasEnabled(pm, pkg, cls, i == 0)) {
                        pm.setComponentEnabledSetting(
                                new ComponentName(pkg, cls),
                                PackageManager.COMPONENT_ENABLED_STATE_DISABLED,
                                PackageManager.DONT_KILL_APP
                        );
                    }
                }
            } catch (Throwable t) {
                Log.w("GlyphTheme", "launcher alias update failed", t);
            }
        });
    }

    private static boolean isAliasEnabled(PackageManager pm, String pkg, String cls, boolean defaultEnabled) {
        int st = pm.getComponentEnabledSetting(new ComponentName(pkg, cls));
        if (st == PackageManager.COMPONENT_ENABLED_STATE_DEFAULT) {
            return defaultEnabled;
        }
        return st == PackageManager.COMPONENT_ENABLED_STATE_ENABLED;
    }
}
