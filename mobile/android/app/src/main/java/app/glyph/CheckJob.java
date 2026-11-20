package app.glyph;

import android.app.job.JobParameters;
import android.app.job.JobService;
import android.util.Log;

/**
 * Runs about twice a day, also with Glyph closed: asks Rust whether there is
 * a new Glyph version or a new release of an artist you follow
 * (`glyph-app/src/notify.rs`) and shows what is new. Rust checks the
 * settings itself; with both kinds off it returns nothing.
 */
public class CheckJob extends JobService {
    static {
        System.loadLibrary("glyph_android");
    }

    /** JSON array of notices; "[]" when there is nothing new. */
    static native String nativeCheck(String filesDir);

    @Override
    public boolean onStartJob(JobParameters params) {
        Thread t = new Thread(() -> {
            try {
                String json = nativeCheck(getFilesDir().getAbsolutePath());
                Notifications.post(getApplicationContext(), json);
            } catch (Throwable e) {
                Log.w("glyph", "check job failed: " + e);
            }
            jobFinished(params, false);
        }, "glyph-check");
        t.start();
        return true;
    }

    @Override
    public boolean onStopJob(JobParameters params) {
        // Stopped early (network gone): try again later.
        return true;
    }
}
