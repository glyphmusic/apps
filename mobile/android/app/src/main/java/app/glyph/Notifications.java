package app.glyph;

import android.Manifest;
import android.app.NotificationChannel;
import android.app.NotificationManager;
import android.app.PendingIntent;
import android.app.job.JobInfo;
import android.app.job.JobScheduler;
import android.content.ComponentName;
import android.content.Context;
import android.content.Intent;
import android.content.pm.PackageManager;
import android.net.Uri;
import android.os.Build;
import android.util.Log;

import org.json.JSONArray;
import org.json.JSONObject;

/**
 * Glyph's own notifications (Rust `glyph-app/src/notify.rs`): "Glyph x.y
 * is ready" and "New from <artist>". Each kind has its own channel, so it
 * can also be turned off in Android's settings; Glyph's settings turn them
 * off at the source. A tap opens the app at the matching page
 * (`glyph://update`, `glyph://album/...`).
 */
final class Notifications {
    private static final String TAG = "glyph";
    /** The check while Glyph is closed ({@link CheckJob}). */
    static final int JOB_ID = 4711;
    private static final long EVERY_MS = 12L * 60 * 60 * 1000;
    private static final long FLEX_MS = 2L * 60 * 60 * 1000;

    private Notifications() {
    }

    /** Shows what Rust sends: one notice (object) or several (array). */
    static void post(Context ctx, String json) {
        if (json == null || json.isEmpty()) {
            return;
        }
        try {
            String t = json.trim();
            if (t.startsWith("[")) {
                JSONArray list = new JSONArray(t);
                for (int i = 0; i < list.length(); i++) {
                    postOne(ctx, list.getJSONObject(i));
                }
            } else if (t.startsWith("{")) {
                postOne(ctx, new JSONObject(t));
            }
        } catch (Exception e) {
            Log.w(TAG, "notification: " + e);
        }
    }

    private static void postOne(Context ctx, JSONObject n) {
        if (Build.VERSION.SDK_INT >= 33
                && ctx.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
            return;
        }
        NotificationManager nm = (NotificationManager) ctx.getSystemService(Context.NOTIFICATION_SERVICE);
        if (nm == null) {
            return;
        }
        String channel = n.optString("channel", "updates");
        if (Build.VERSION.SDK_INT >= 26) {
            String name = n.optString("channel_name", channel);
            NotificationChannel ch = nm.getNotificationChannel(channel);
            if (ch == null || !name.equals(String.valueOf(ch.getName()))) {
                nm.createNotificationChannel(new NotificationChannel(channel, name, NotificationManager.IMPORTANCE_DEFAULT));
            }
        }
        int id = n.optInt("id", 1);
        Intent open = new Intent(ctx, MainActivity.class)
                .setAction(Intent.ACTION_VIEW)
                .setData(Uri.parse(n.optString("url", "glyph://open")))
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK | Intent.FLAG_ACTIVITY_SINGLE_TOP);
        PendingIntent tap = PendingIntent.getActivity(ctx, id, open,
                PendingIntent.FLAG_IMMUTABLE | PendingIntent.FLAG_UPDATE_CURRENT);
        android.app.Notification.Builder b = new android.app.Notification.Builder(ctx, channel);
        String body = n.optString("body", "");
        b.setSmallIcon(R.drawable.ic_stat_glyph)
                .setContentTitle(n.optString("title", "Glyph"))
                .setContentText(body)
                .setStyle(new android.app.Notification.BigTextStyle().bigText(body))
                .setContentIntent(tap)
                .setAutoCancel(true)
                .setColor(0xFFF472B6);
        try {
            nm.notify(channel, id, b.build());
        } catch (SecurityException e) {
            Log.w(TAG, "notification: " + e);
        }
    }

    /** Asks Android to run {@link CheckJob} about twice a day (once). */
    static void schedule(Context ctx) {
        try {
            JobScheduler js = (JobScheduler) ctx.getSystemService(Context.JOB_SCHEDULER_SERVICE);
            if (js == null) {
                return;
            }
            if (Build.VERSION.SDK_INT >= 24 && js.getPendingJob(JOB_ID) != null) {
                return;
            }
            JobInfo job = new JobInfo.Builder(JOB_ID, new ComponentName(ctx, CheckJob.class))
                    .setRequiredNetworkType(JobInfo.NETWORK_TYPE_ANY)
                    .setPeriodic(EVERY_MS, FLEX_MS)
                    .build();
            js.schedule(job);
        } catch (Exception e) {
            Log.w(TAG, "check job: " + e);
        }
    }
}
