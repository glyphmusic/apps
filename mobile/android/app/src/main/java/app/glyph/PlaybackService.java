package app.glyph;

import android.app.Notification;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.Service;
import android.content.Context;
import android.content.Intent;
import android.content.pm.ServiceInfo;
import android.graphics.Bitmap;
import android.graphics.BitmapFactory;
import android.media.AudioAttributes;
import android.media.AudioFocusRequest;
import android.media.AudioManager;
import android.media.AudioPlaybackConfiguration;
import android.net.wifi.WifiManager;
import android.os.Build;
import android.os.Handler;
import android.os.IBinder;
import android.os.Looper;
import android.os.PowerManager;
import android.os.SystemClock;
import android.support.v4.media.MediaMetadataCompat;
import android.support.v4.media.session.MediaSessionCompat;
import android.support.v4.media.session.PlaybackStateCompat;

import androidx.core.app.NotificationCompat;
import androidx.media.VolumeProviderCompat;
import androidx.media.app.NotificationCompat.MediaStyle;
import androidx.media.session.MediaButtonReceiver;

import java.util.List;
import java.util.Objects;

/**
 * Keeps Glyph alive while it plays in the background and shows the media
 * notification / lock-screen controls. Audio itself plays in Rust (AAudio);
 * this only mirrors the state and forwards button presses.
 *
 * The song's cover is the session's artwork (Android draws it behind the
 * media controls) and an add button puts the song into Liked Songs, as in
 * Spotify. Rust sends the position every few seconds; only what changed is
 * handed to the system (a new bitmap or notification on every update made
 * the system UI work for nothing).
 *
 * While it plays the service stays in the foreground and holds a CPU and a
 * Wi-Fi lock: between two songs (finding and buffering the next one) and
 * while a network device plays, no audio runs here to keep the phone
 * awake, and a cached app gets frozen. A pause gives up the foreground only
 * after a moment, so a short stop between songs never loses it (getting it
 * back from the background is not allowed on newer Android).
 *
 * While a network device plays, the session is a remote one: the volume
 * keys (also on the lock screen) set the device's volume.
 *
 * Audio focus ("Pause when other apps play"): while Glyph plays it holds
 * the focus, so another app that starts playing takes it and Rust pauses
 * (glyph_app::other_audio). A short interruption (a voice message, a
 * navigation prompt) hands the focus back when it ends. A video or another
 * music app keeps it: then the other players are counted (beyond Glyph's
 * own, which stays open while paused) to see when the other sound stops,
 * and whether it is music (no playing on after a music app).
 */
public class PlaybackService extends Service {

    static {
        System.loadLibrary("glyph_android");
    }

    private static final String CHANNEL = "playback";
    private static final int NOTIFICATION_ID = 1;
    /** Custom action / intent action of the add (like) button. */
    private static final String ACTION_LIKE = "app.glyph.LIKE";
    /** Longest side of the artwork bitmap (the system scales it anyway). */
    private static final int ART_PX = 720;

    // Codes for nativeMediaKey (see glyph-android: nativeMediaKey).
    static final int KEY_PLAY = 0, KEY_PAUSE = 1, KEY_TOGGLE = 2, KEY_NEXT = 3, KEY_PREVIOUS = 4, KEY_STOP = 5,
            KEY_LIKE = 6;

    static native void nativeMediaKey(int code);

    /** Volume 0..100 for the network device playing now. */
    static native void nativeVolume(int volume);

    /** Another app plays sound now (and whether it is music); see glyph_app::other_audio. */
    static native void nativeOtherAudio(boolean active, boolean music);

    /** Setting "Pause when other apps play" (from Rust). Off: no audio focus, Glyph plays along. */
    private static boolean pauseForOthers = true;
    /** After the focus went to another app, it counts as playing this long even before its player shows up. */
    private static final long OTHER_GRACE_MS = 5_000;
    /** How long the other players are watched after the focus is gone (Rust stops playing on after 5 minutes). */
    private static final long OTHER_WATCH_MS = 6 * 60_000;
    private static final long OTHER_POLL_MS = 2_000;

    /** How long a pause keeps the foreground (and the locks). */
    private static final long PAUSE_GRACE_MS = 15_000;
    /** Volume steps of the device (5 % each, as Android does for Cast). */
    private static final int VOLUME_STEPS = 20, VOLUME_STEP = 100 / VOLUME_STEPS;
    /** After a volume key, the device's echo of an older volume is ignored this long. */
    private static final long VOLUME_SETTLE_MS = 1_500;

    private static String title = "", artist = "", coverPath = "";
    private static boolean playing = false, buffering = false, liked = false, jamActive = false;
    private static long positionMs = 0, durationMs = 0;
    /** Volume of the network device playing now, -1 when the phone plays. */
    private static int remoteVolume = -1;
    private static long volumeKeyAt;

    /** The running service (main thread only). */
    private static PlaybackService instance;

    private MediaSessionCompat session;
    private Bitmap art;
    private String artPath = "";
    private String metaKey, notificationKey;
    private boolean foreground;
    private VolumeProviderCompat volumeProvider;
    private PowerManager.WakeLock wakeLock;
    private WifiManager.WifiLock wifiLock;
    private final Handler handler = new Handler(Looper.getMainLooper());
    private final Runnable leaveForeground = this::leaveForeground;

    private AudioManager audio;
    private AudioFocusRequest focusRequest;
    /** Glyph is in the focus stack (also during a short interruption). */
    private boolean hasFocus;
    /** Another app has the focus now. */
    private boolean focusLost;
    /** The focus went away for good (a video, another music app): watch the players. */
    private boolean focusGone;
    private long lostAt;
    /** Players (all / music) while Glyph plays alone: its own. */
    private int ownPlayers = 1, ownMusic = 1;
    private AudioManager.AudioPlaybackCallback playbackCallback;
    private final Runnable watchOthers = this::watchOthers;

    /**
     * From Rust (MainActivity.gpuiNowPlaying), on the main thread: start,
     * update or stop the service. remote: the network device's volume, -1
     * when the phone plays.
     */
    static void update(Context ctx, String t, String a, boolean p, boolean loading, long pos, long dur, String cover, boolean like,
                       int remote) {
        boolean has = t != null && !t.isEmpty();
        title = has ? t : "";
        artist = a == null ? "" : a;
        playing = p;
        buffering = loading;
        positionMs = pos;
        durationMs = dur;
        coverPath = cover == null ? "" : cover;
        liked = like;
        if (remote < 0 || SystemClock.uptimeMillis() - volumeKeyAt > VOLUME_SETTLE_MS) {
            remoteVolume = remote;
        }
        syncService(ctx);
    }

    /** From Rust: the setting "Pause when other apps play". */
    static void setPauseForOthers(boolean on) {
        if (pauseForOthers == on) {
            return;
        }
        pauseForOthers = on;
        if (instance != null) {
            instance.syncFocus();
        }
    }

    /** Keeps the service running in the foreground with CPU/Wi-Fi locks whenever a Jam is active. */
    static void setJamActive(Context ctx, boolean active) {
        if (jamActive == active) {
            return;
        }
        jamActive = active;
        syncService(ctx);
    }

    private static void syncService(Context ctx) {
        Intent i = new Intent(ctx, PlaybackService.class);
        if (title.isEmpty() && !jamActive) {
            ctx.stopService(i);
            return;
        }
        if (instance != null) {
            instance.apply(false);
            return;
        }
        try {
            if (Build.VERSION.SDK_INT >= 26) {
                ctx.startForegroundService(i);
            } else {
                ctx.startService(i);
            }
        } catch (Exception ignored) {
            // Starting a foreground service from the background is not
            // allowed on newer Android; the next update from the foreground
            // starts it.
        }
    }

    /** The volume keys control a network device right now. */
    static boolean remoteVolumeActive() {
        return remoteVolume >= 0;
    }

    /**
     * A volume key while the activity is in front (direction +1 / -1). Goes
     * through the session, so the system shows its volume panel for the
     * device; without the service the device gets the new volume directly.
     */
    static void volumeKey(int direction) {
        if (remoteVolume < 0) {
            return;
        }
        if (instance != null && instance.volumeProvider != null) {
            instance.session.getController().adjustVolume(direction, AudioManager.FLAG_SHOW_UI);
        } else {
            setRemoteStep(Math.round(remoteVolume / (float) VOLUME_STEP) + direction);
        }
    }

    private static void setRemoteStep(int step) {
        int s = Math.max(0, Math.min(VOLUME_STEPS, step));
        remoteVolume = s * VOLUME_STEP;
        volumeKeyAt = SystemClock.uptimeMillis();
        if (instance != null && instance.volumeProvider != null) {
            instance.volumeProvider.setCurrentVolume(s);
        }
        nativeVolume(remoteVolume);
    }

    @Override
    public void onCreate() {
        super.onCreate();
        instance = this;
        if (Build.VERSION.SDK_INT >= 26) {
            NotificationChannel ch = new NotificationChannel(CHANNEL, "Playback", NotificationManager.IMPORTANCE_LOW);
            ch.setShowBadge(false);
            getSystemService(NotificationManager.class).createNotificationChannel(ch);
        }
        session = new MediaSessionCompat(this, "Glyph");
        session.setCallback(new MediaSessionCompat.Callback() {
            @Override public void onPlay() {
                promoteFromMediaAction();
                nativeMediaKey(KEY_PLAY);
            }
            @Override public void onPause() { nativeMediaKey(KEY_PAUSE); }
            @Override public void onSkipToNext() {
                promoteFromMediaAction();
                nativeMediaKey(KEY_NEXT);
            }
            @Override public void onSkipToPrevious() {
                promoteFromMediaAction();
                nativeMediaKey(KEY_PREVIOUS);
            }
            @Override public void onStop() { nativeMediaKey(KEY_STOP); }
            @Override public void onCustomAction(String action, android.os.Bundle extras) {
                if (ACTION_LIKE.equals(action)) {
                    nativeMediaKey(KEY_LIKE);
                }
            }
        });
        session.setActive(true);
        audio = getSystemService(AudioManager.class);
        playbackCallback = new AudioManager.AudioPlaybackCallback() {
            @Override
            public void onPlaybackConfigChanged(List<AudioPlaybackConfiguration> configs) {
                countPlayers(configs);
            }
        };
        try {
            audio.registerAudioPlaybackCallback(playbackCallback, handler);
        } catch (Exception ignored) {
            playbackCallback = null;
        }
    }

    // ---- Audio focus (see the class comment) ----

    /** Takes or gives up the focus as playback and the setting say. */
    private void syncFocus() {
        if (audio == null) {
            return;
        }
        boolean want = pauseForOthers && remoteVolume < 0 && (playing || buffering);
        if (!pauseForOthers || remoteVolume >= 0) {
            abandonFocus();
        } else if (want && !hasFocus) {
            if (focusRequest == null) {
                focusRequest = new AudioFocusRequest.Builder(AudioManager.AUDIOFOCUS_GAIN)
                        .setAudioAttributes(new AudioAttributes.Builder()
                                .setUsage(AudioAttributes.USAGE_MEDIA)
                                .setContentType(AudioAttributes.CONTENT_TYPE_MUSIC)
                                .build())
                        // Rust pauses itself; ducking stays the system's job.
                        .setWillPauseWhenDucked(false)
                        .setOnAudioFocusChangeListener(this::onFocusChange, handler)
                        .build();
            }
            int r;
            try {
                r = audio.requestAudioFocus(focusRequest);
            } catch (Exception e) {
                r = AudioManager.AUDIOFOCUS_REQUEST_FAILED;
            }
            if (r == AudioManager.AUDIOFOCUS_REQUEST_GRANTED) {
                hasFocus = true;
                otherStopped();
            }
        }
    }

    private void abandonFocus() {
        if (hasFocus && audio != null && focusRequest != null) {
            try {
                audio.abandonAudioFocusRequest(focusRequest);
            } catch (Exception ignored) {
                // Gone anyway.
            }
        }
        hasFocus = false;
        otherStopped();
    }

    private void otherStopped() {
        handler.removeCallbacks(watchOthers);
        if (focusLost) {
            focusLost = false;
            focusGone = false;
            nativeOtherAudio(false, false);
        }
    }

    private void onFocusChange(int change) {
        switch (change) {
            case AudioManager.AUDIOFOCUS_GAIN:
                hasFocus = true;
                otherStopped();
                break;
            case AudioManager.AUDIOFOCUS_LOSS_TRANSIENT:
                // Comes back with AUDIOFOCUS_GAIN when the other app is done.
                focusLost = true;
                focusGone = false;
                nativeOtherAudio(true, false);
                break;
            case AudioManager.AUDIOFOCUS_LOSS:
                // For good: Android will not hand it back, so watch the players.
                hasFocus = false;
                focusLost = true;
                focusGone = true;
                lostAt = SystemClock.uptimeMillis();
                nativeOtherAudio(true, false);
                handler.removeCallbacks(watchOthers);
                handler.postDelayed(watchOthers, OTHER_GRACE_MS);
                break;
            default:
                // AUDIOFOCUS_LOSS_TRANSIENT_CAN_DUCK: the system turns Glyph down.
                break;
        }
    }

    /** Every few seconds after the focus went away for good. */
    private void watchOthers() {
        if (!focusGone) {
            return;
        }
        try {
            countPlayers(audio.getActivePlaybackConfigurations());
        } catch (Exception ignored) {
            // Try again next time.
        }
        if (SystemClock.uptimeMillis() - lostAt < OTHER_WATCH_MS) {
            handler.postDelayed(watchOthers, OTHER_POLL_MS);
        } else {
            otherStopped();
        }
    }

    /** The players playing now: Glyph's own while it plays alone, the other app's after the focus went away. */
    private void countPlayers(List<AudioPlaybackConfiguration> configs) {
        if (configs == null) {
            return;
        }
        int all = 0, music = 0;
        for (AudioPlaybackConfiguration c : configs) {
            AudioAttributes a = c.getAudioAttributes();
            int usage = a.getUsage();
            if (usage != AudioAttributes.USAGE_MEDIA && usage != AudioAttributes.USAGE_GAME
                    && usage != AudioAttributes.USAGE_UNKNOWN) {
                continue;
            }
            all++;
            if (a.getContentType() == AudioAttributes.CONTENT_TYPE_MUSIC) {
                music++;
            }
        }
        if (hasFocus && !focusLost && playing && !buffering) {
            ownPlayers = all;
            ownMusic = music;
        } else if (focusGone && SystemClock.uptimeMillis() - lostAt >= OTHER_GRACE_MS) {
            nativeOtherAudio(all > ownPlayers, music > ownMusic);
        }
    }

    /**
     * Promotes to foreground synchronously inside a MediaSession user callback so Android 12+
     * does not reject background startForeground/network work before Rust ticks.
     */
    private void promoteFromMediaAction() {
        if (!foreground && !title.isEmpty()) {
            buffering = true;
            apply(true);
        }
    }

    @Override
    public int onStartCommand(Intent intent, int flags, int startId) {
        if (intent != null && ACTION_LIKE.equals(intent.getAction())) {
            // The add button of the notification itself (Android 12 and older).
            nativeMediaKey(KEY_LIKE);
        } else {
            MediaButtonReceiver.handleIntent(session, intent);
        }
        // startForegroundService wants startForeground, also when paused.
        apply(true);
        return START_NOT_STICKY;
    }

    /** Hands the current state to the session and the notification. */
    private void apply(boolean mustStartForeground) {
        applyVolume();
        syncFocus();
        String meta = title + '\u0000' + artist + '\u0000' + durationMs + '\u0000' + coverPath;
        if (!meta.equals(metaKey)) {
            metaKey = meta;
            MediaMetadataCompat.Builder m = new MediaMetadataCompat.Builder()
                    .putString(MediaMetadataCompat.METADATA_KEY_TITLE, title)
                    .putString(MediaMetadataCompat.METADATA_KEY_ARTIST, artist)
                    .putLong(MediaMetadataCompat.METADATA_KEY_DURATION, durationMs);
            Bitmap cover = artwork();
            if (cover != null) {
                m.putBitmap(MediaMetadataCompat.METADATA_KEY_ALBUM_ART, cover);
            }
            session.setMetadata(m.build());
        }
        int likeIcon = liked ? R.drawable.ic_liked : R.drawable.ic_like;
        String likeLabel = liked ? "Remove from Liked Songs" : "Add to Liked Songs";
        session.setPlaybackState(new PlaybackStateCompat.Builder()
                .setActions(PlaybackStateCompat.ACTION_PLAY | PlaybackStateCompat.ACTION_PAUSE
                        | PlaybackStateCompat.ACTION_PLAY_PAUSE | PlaybackStateCompat.ACTION_SKIP_TO_NEXT
                        | PlaybackStateCompat.ACTION_SKIP_TO_PREVIOUS | PlaybackStateCompat.ACTION_STOP)
                // Android 13+ draws its media controls from the session: the
                // custom action is the add button left of "previous".
                .addCustomAction(new PlaybackStateCompat.CustomAction.Builder(ACTION_LIKE, likeLabel, likeIcon).build())
                .setState(buffering ? PlaybackStateCompat.STATE_BUFFERING
                        : playing ? PlaybackStateCompat.STATE_PLAYING : PlaybackStateCompat.STATE_PAUSED,
                        positionMs, playing && !buffering ? 1f : 0f)
                .build());

        boolean active = playing || buffering || jamActive;
        String key = meta + '\u0000' + playing + '\u0000' + buffering + '\u0000' + liked + '\u0000' + jamActive;
        // Paused (and not in a Jam): the grace timer already runs (or the foreground is gone).
        if (!mustStartForeground && key.equals(notificationKey) && (foreground || !active)) {
            return;
        }
        notificationKey = key;
        Notification n = buildNotification(likeIcon, likeLabel);
        if (active || mustStartForeground) {
            handler.removeCallbacks(leaveForeground);
            try {
                if (Build.VERSION.SDK_INT >= 29) {
                    startForeground(NOTIFICATION_ID, n, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PLAYBACK);
                } else {
                    startForeground(NOTIFICATION_ID, n);
                }
                foreground = true;
            } catch (Exception e) {
                // Not allowed from the background on newer Android: at least
                // keep the notification current.
                getSystemService(NotificationManager.class).notify(NOTIFICATION_ID, n);
            }
            holdLocks(true);
        }
        if (!active) {
            getSystemService(NotificationManager.class).notify(NOTIFICATION_ID, n);
            if (foreground) {
                // Paused, or between two songs: let go a bit later.
                handler.removeCallbacks(leaveForeground);
                handler.postDelayed(leaveForeground, PAUSE_GRACE_MS);
            } else {
                holdLocks(false);
            }
        }
    }

    /** Still paused after the grace time: the notification may be swiped away, the phone may sleep. */
    private void leaveForeground() {
        if (playing || buffering || jamActive) {
            return;
        }
        if (foreground) {
            stopForeground(STOP_FOREGROUND_DETACH);
            foreground = false;
        }
        holdLocks(false);
        // Paused for a while: the focus is free for other apps again.
        if (!focusLost) {
            abandonFocus();
        }
    }

    /** Keeps the CPU and Wi-Fi awake while playing (network streams, device control). */
    @SuppressWarnings("deprecation")
    private void holdLocks(boolean hold) {
        try {
            if (hold) {
                if (wakeLock == null) {
                    wakeLock = getSystemService(PowerManager.class).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "glyph:playback");
                    wakeLock.setReferenceCounted(false);
                }
                if (!wakeLock.isHeld()) {
                    wakeLock.acquire();
                }
                if (wifiLock == null) {
                    WifiManager wm = (WifiManager) getApplicationContext().getSystemService(Context.WIFI_SERVICE);
                    if (wm != null) {
                        int mode = Build.VERSION.SDK_INT >= 29
                                ? WifiManager.WIFI_MODE_FULL_LOW_LATENCY
                                : WifiManager.WIFI_MODE_FULL_HIGH_PERF;
                        wifiLock = wm.createWifiLock(mode, "glyph:playback");
                        wifiLock.setReferenceCounted(false);
                    }
                }
                if (wifiLock != null && !wifiLock.isHeld()) {
                    wifiLock.acquire();
                }
            } else {
                if (wakeLock != null && wakeLock.isHeld()) {
                    wakeLock.release();
                }
                if (wifiLock != null && wifiLock.isHeld()) {
                    wifiLock.release();
                }
            }
        } catch (Exception ignored) {
            // Locks are a help, not a must.
        }
    }

    /** A network device plays: a remote session whose volume is the device's. */
    private void applyVolume() {
        if (remoteVolume < 0) {
            if (volumeProvider != null) {
                volumeProvider = null;
                session.setPlaybackToLocal(AudioManager.STREAM_MUSIC);
            }
            return;
        }
        int step = Math.round(remoteVolume / (float) VOLUME_STEP);
        if (volumeProvider == null) {
            volumeProvider = new VolumeProviderCompat(VolumeProviderCompat.VOLUME_CONTROL_ABSOLUTE, VOLUME_STEPS, step) {
                @Override
                public void onSetVolumeTo(int volume) {
                    setRemoteStep(volume);
                }

                @Override
                public void onAdjustVolume(int direction) {
                    if (direction == AudioManager.ADJUST_RAISE || direction == AudioManager.ADJUST_LOWER) {
                        setRemoteStep(getCurrentVolume() + direction);
                    } else if (direction == AudioManager.ADJUST_MUTE) {
                        setRemoteStep(0);
                    }
                }
            };
            session.setPlaybackToRemote(volumeProvider);
        } else if (step != volumeProvider.getCurrentVolume()) {
            volumeProvider.setCurrentVolume(step);
        }
    }

    /** The cover file as a bitmap, decoded once per song. */
    private Bitmap artwork() {
        if (Objects.equals(coverPath, artPath)) {
            return art;
        }
        artPath = coverPath;
        art = null;
        if (coverPath.isEmpty()) {
            return null;
        }
        try {
            BitmapFactory.Options bounds = new BitmapFactory.Options();
            bounds.inJustDecodeBounds = true;
            BitmapFactory.decodeFile(coverPath, bounds);
            int sample = 1;
            while (Math.max(bounds.outWidth, bounds.outHeight) / (sample * 2) >= ART_PX) {
                sample *= 2;
            }
            BitmapFactory.Options opts = new BitmapFactory.Options();
            opts.inSampleSize = sample;
            Bitmap decoded = BitmapFactory.decodeFile(coverPath, opts);
            int side = decoded == null ? 0 : Math.max(decoded.getWidth(), decoded.getHeight());
            if (side > ART_PX) {
                float k = ART_PX / (float) side;
                art = Bitmap.createScaledBitmap(decoded, Math.max(1, Math.round(decoded.getWidth() * k)),
                        Math.max(1, Math.round(decoded.getHeight() * k)), true);
                decoded.recycle();
            } else {
                art = decoded;
            }
        } catch (Exception | OutOfMemoryError ignored) {
            art = null;
        }
        return art;
    }

    private Notification buildNotification(int likeIcon, String likeLabel) {
        Intent open = new Intent(this, MainActivity.class).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent content = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        if (title.isEmpty()) {
            return new NotificationCompat.Builder(this, CHANNEL)
                    .setSmallIcon(R.drawable.ic_stat_glyph)
                    .setContentTitle("Glyph Jam")
                    .setContentText("Jam active")
                    .setContentIntent(content)
                    .setOngoing(true)
                    .setOnlyAlertOnce(true)
                    .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
                    .build();
        }
        Intent likeIntent = new Intent(this, PlaybackService.class).setAction(ACTION_LIKE);
        NotificationCompat.Action like = new NotificationCompat.Action(likeIcon, likeLabel,
                PendingIntent.getService(this, 1, likeIntent, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
        NotificationCompat.Action prev = new NotificationCompat.Action(android.R.drawable.ic_media_previous, "Previous",
                MediaButtonReceiver.buildMediaButtonPendingIntent(this, PlaybackStateCompat.ACTION_SKIP_TO_PREVIOUS));
        NotificationCompat.Action toggle = playing || buffering
                ? new NotificationCompat.Action(android.R.drawable.ic_media_pause, "Pause",
                        MediaButtonReceiver.buildMediaButtonPendingIntent(this, PlaybackStateCompat.ACTION_PAUSE))
                : new NotificationCompat.Action(android.R.drawable.ic_media_play, "Play",
                        MediaButtonReceiver.buildMediaButtonPendingIntent(this, PlaybackStateCompat.ACTION_PLAY));
        NotificationCompat.Action next = new NotificationCompat.Action(android.R.drawable.ic_media_next, "Next",
                MediaButtonReceiver.buildMediaButtonPendingIntent(this, PlaybackStateCompat.ACTION_SKIP_TO_NEXT));
        return new NotificationCompat.Builder(this, CHANNEL)
                .setSmallIcon(R.drawable.ic_stat_glyph)
                .setContentTitle(title)
                .setContentText(artist)
                .setLargeIcon(artwork())
                .setContentIntent(content)
                .setOngoing(playing || buffering)
                .setOnlyAlertOnce(true)
                .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
                .addAction(like)
                .addAction(prev)
                .addAction(toggle)
                .addAction(next)
                .setStyle(new MediaStyle().setMediaSession(session.getSessionToken()).setShowActionsInCompactView(1, 2, 3))
                .build();
    }

    @Override
    public void onDestroy() {
        instance = null;
        handler.removeCallbacks(leaveForeground);
        holdLocks(false);
        abandonFocus();
        if (playbackCallback != null) {
            audio.unregisterAudioPlaybackCallback(playbackCallback);
        }
        session.setActive(false);
        session.release();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
