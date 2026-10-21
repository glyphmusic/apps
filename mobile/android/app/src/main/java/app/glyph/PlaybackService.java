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
import android.os.Build;
import android.os.IBinder;
import android.support.v4.media.MediaMetadataCompat;
import android.support.v4.media.session.MediaSessionCompat;
import android.support.v4.media.session.PlaybackStateCompat;

import androidx.core.app.NotificationCompat;
import androidx.media.app.NotificationCompat.MediaStyle;
import androidx.media.session.MediaButtonReceiver;

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
 */
public class PlaybackService extends Service {

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

    private static String title = "", artist = "", coverPath = "";
    private static boolean playing = false, liked = false;
    private static long positionMs = 0, durationMs = 0;

    /** The running service (main thread only). */
    private static PlaybackService instance;

    private MediaSessionCompat session;
    private Bitmap art;
    private String artPath = "";
    private String metaKey, notificationKey;
    private boolean foreground;

    /** From Rust (MainActivity.gpuiNowPlaying), on the main thread: start, update or stop the service. */
    static void update(Context ctx, String t, String a, boolean p, long pos, long dur, String cover, boolean like) {
        boolean has = t != null && !t.isEmpty();
        title = has ? t : "";
        artist = a == null ? "" : a;
        playing = p;
        positionMs = pos;
        durationMs = dur;
        coverPath = cover == null ? "" : cover;
        liked = like;
        Intent i = new Intent(ctx, PlaybackService.class);
        if (!has) {
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
            @Override public void onPlay() { nativeMediaKey(KEY_PLAY); }
            @Override public void onPause() { nativeMediaKey(KEY_PAUSE); }
            @Override public void onSkipToNext() { nativeMediaKey(KEY_NEXT); }
            @Override public void onSkipToPrevious() { nativeMediaKey(KEY_PREVIOUS); }
            @Override public void onStop() { nativeMediaKey(KEY_STOP); }
            @Override public void onCustomAction(String action, android.os.Bundle extras) {
                if (ACTION_LIKE.equals(action)) {
                    nativeMediaKey(KEY_LIKE);
                }
            }
        });
        session.setActive(true);
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
                .setState(playing ? PlaybackStateCompat.STATE_PLAYING : PlaybackStateCompat.STATE_PAUSED, positionMs, playing ? 1f : 0f)
                .build());

        String key = meta + '\u0000' + playing + '\u0000' + liked;
        if (!mustStartForeground && key.equals(notificationKey) && foreground == playing) {
            return;
        }
        notificationKey = key;
        Notification n = buildNotification(likeIcon, likeLabel);
        if (playing || mustStartForeground) {
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
        }
        if (!playing) {
            if (foreground) {
                // Paused: the notification may be swiped away, the process may sleep.
                stopForeground(STOP_FOREGROUND_DETACH);
                foreground = false;
            }
            getSystemService(NotificationManager.class).notify(NOTIFICATION_ID, n);
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
        Intent likeIntent = new Intent(this, PlaybackService.class).setAction(ACTION_LIKE);
        NotificationCompat.Action like = new NotificationCompat.Action(likeIcon, likeLabel,
                PendingIntent.getService(this, 1, likeIntent, PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT));
        NotificationCompat.Action prev = new NotificationCompat.Action(android.R.drawable.ic_media_previous, "Previous",
                MediaButtonReceiver.buildMediaButtonPendingIntent(this, PlaybackStateCompat.ACTION_SKIP_TO_PREVIOUS));
        NotificationCompat.Action toggle = playing
                ? new NotificationCompat.Action(android.R.drawable.ic_media_pause, "Pause",
                        MediaButtonReceiver.buildMediaButtonPendingIntent(this, PlaybackStateCompat.ACTION_PAUSE))
                : new NotificationCompat.Action(android.R.drawable.ic_media_play, "Play",
                        MediaButtonReceiver.buildMediaButtonPendingIntent(this, PlaybackStateCompat.ACTION_PLAY));
        NotificationCompat.Action next = new NotificationCompat.Action(android.R.drawable.ic_media_next, "Next",
                MediaButtonReceiver.buildMediaButtonPendingIntent(this, PlaybackStateCompat.ACTION_SKIP_TO_NEXT));
        return new NotificationCompat.Builder(this, CHANNEL)
                .setSmallIcon(android.R.drawable.ic_media_play)
                .setContentTitle(title)
                .setContentText(artist)
                .setLargeIcon(artwork())
                .setContentIntent(content)
                .setOngoing(playing)
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
        session.setActive(false);
        session.release();
        super.onDestroy();
    }

    @Override
    public IBinder onBind(Intent intent) {
        return null;
    }
}
