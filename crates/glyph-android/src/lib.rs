//! Android entry point. GameActivity starts `android_main` on its own thread;
//! from there on it is the same app as on the desktop: `glyph::init` and
//! `glyph::root` build the views, only the platform underneath differs.
#![cfg(target_os = "android")]

use gpui::WindowOptions;
use gpui_android::android_activity::AndroidApp;
use glyph_audio::MediaKeyEvent;
use jni::objects::{JClass, JObject, JString};
use jni::sys::jint;
use jni::{jni_sig, jni_str, EnvUnowned, JValue};

#[unsafe(no_mangle)]
fn android_main(app: AndroidApp) {
    android_logger::init_once(
        android_logger::Config::default().with_max_level(log::LevelFilter::Info).with_tag("glyph"),
    );
    std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));

    // dirs:: knows no Android folders: point the core at the app's own.
    if let Some(base) = app.internal_data_path() {
        let set = |key: &str, dir: std::path::PathBuf| {
            let _ = std::fs::create_dir_all(&dir);
            // Before any other thread of ours exists.
            std::env::set_var(key, dir);
        };
        set("GLYPH_DATA_DIR", base.join("data"));
        set("GLYPH_CACHE_DIR", base.join("cache"));
        let config = base.join("config");
        let _ = std::fs::create_dir_all(&config);
        std::env::set_var("GLYPH_CONFIG", config.join("config.toml"));
    }

    // `adb shell setprop debug.glyph.gpu gl`: use the OpenGL ES adapter
    // instead of Vulkan (diagnosing GPU driver problems, e.g. on emulators).
    if getprop("debug.glyph.gpu").as_deref() == Some("gl") {
        std::env::set_var("ZED_DEVICE_ID", "0x0000");
    }

    // Background playback: now playing goes to MainActivity.gpuiNowPlaying,
    // which runs the media notification service (PlaybackService.java).
    // The cover file becomes the notification's artwork ("" for none),
    // `liked` the state of its add button, and the remote volume (-1: none)
    // hands the volume keys to a network device.
    glyph_audio::media_keys::set_now_playing_sink(Box::new(|now: &glyph_audio::media_keys::NowPlaying<'_>| {
        let (title, artist, dur) = match now.track {
            Some(t) => (t.title.clone(), t.artist.clone(), t.duration_ms.unwrap_or(0)),
            None => (String::new(), String::new(), 0),
        };
        let cover = now.cover.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let remote_volume = now.remote_volume.map_or(-1, i32::from);
        gpui_android::java::with_activity("now playing", |env, activity| {
            let title = env.new_string(&title)?;
            let artist = env.new_string(&artist)?;
            let cover = env.new_string(&cover)?;
            env.call_method(
                activity,
                jni_str!("gpuiNowPlaying"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;ZJJLjava/lang/String;ZI)V"),
                &[
                    JValue::Object(&title),
                    JValue::Object(&artist),
                    JValue::Bool(now.playing.into()),
                    JValue::Long(now.position.as_millis() as i64),
                    JValue::Long(dur as i64),
                    JValue::Object(&cover),
                    JValue::Bool(now.liked.into()),
                    JValue::Int(remote_volume),
                ],
            )?;
            Ok(())
        });
    }));

    // In-app updates: the verified APK goes to MainActivity.gpuiInstallApk
    // (install permission, then the system installer's confirmation).
    glyph_core::updater::set_apk_installer(Box::new(|path| {
        let path = path.to_string_lossy().into_owned();
        gpui_android::java::with_activity("install update", |env, activity| {
            let path = env.new_string(&path)?;
            env.call_method(activity, jni_str!("gpuiInstallApk"), jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&path)])?;
            Ok(())
        })
        .is_some()
    }));

    // ANDROID_ID for the hardware id servers can ban (glyph_core::hardware).
    std::thread::spawn(|| {
        for _ in 0..100 {
            let id = gpui_android::java::with_activity("android id", |env, activity| {
                let value = env.call_method(activity, jni_str!("gpuiAndroidId"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
                if value.is_null() {
                    return Ok(None);
                }
                let value = env.cast_local::<JString>(value)?;
                Ok(Some(value.try_to_string(env)?))
            });
            match id {
                Some(Some(id)) => return glyph_core::hardware::set_platform_id(&id),
                Some(None) => return,
                // No activity yet.
                None => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        }
    });

    let config = glyph_core::Config::load().unwrap_or_default();
    glyph::i18n::set_lang(glyph::i18n::Lang::system());
    gpui_android::application(app).with_assets(glyph::assets::Assets).run(move |cx| {
        glyph::init(cx, &config);
        let options = WindowOptions { titlebar: None, ..Default::default() };
        if let Err(e) = cx.open_window(options, |window, cx| glyph::root(window, config, cx)) {
            log::error!("could not open the window: {e:#}");
        }
    });
}

fn getprop(name: &str) -> Option<String> {
    let out = std::process::Command::new("getprop").arg(name).output().ok()?;
    let v = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!v.is_empty()).then_some(v)
}

/// `MainActivity.nativeInsets`: system bars + cutout and keyboard, in device pixels.
#[unsafe(no_mangle)]
extern "system" fn Java_app_glyph_MainActivity_nativeInsets<'local>(
    _env: EnvUnowned<'local>,
    _this: JObject<'local>,
    top: jint,
    right: jint,
    bottom: jint,
    left: jint,
    ime_top: jint,
    ime_right: jint,
    ime_bottom: jint,
    ime_left: jint,
) {
    gpui_android::set_system_insets([top, right, bottom, left], [ime_top, ime_right, ime_bottom, ime_left]);
}

/// `MainActivity.nativeUrlWaiting`: the app was opened with a `glyph://`
/// link (jam invite, playlist). Taken on another thread: the activity may
/// not be handed to Rust yet right after start.
#[unsafe(no_mangle)]
extern "system" fn Java_app_glyph_MainActivity_nativeUrlWaiting<'local>(_env: EnvUnowned<'local>, _this: JObject<'local>) {
    std::thread::spawn(|| {
        for _ in 0..100 {
            let url = gpui_android::java::with_activity("take open url", |env, activity| {
                let value = env.call_method(activity, jni_str!("gpuiTakeOpenUrl"), jni_sig!("()Ljava/lang/String;"), &[])?.l()?;
                if value.is_null() {
                    return Ok(None);
                }
                let value = env.cast_local::<JString>(value)?;
                Ok(Some(value.try_to_string(env)?))
            });
            match url {
                Some(Some(url)) => {
                    glyph::instance::open_url(&url);
                    gpui_android::wake();
                    return;
                }
                // Taken already (or none).
                Some(None) => return,
                // No activity yet.
                None => std::thread::sleep(std::time::Duration::from_millis(100)),
            }
        }
    });
}

/// `PlaybackService.nativeMediaKey`: a button on the notification, lock
/// screen or headset (codes in PlaybackService.java).
#[unsafe(no_mangle)]
extern "system" fn Java_app_glyph_PlaybackService_nativeMediaKey<'local>(_env: EnvUnowned<'local>, _class: JClass<'local>, code: jint) {
    let ev = match code {
        0 => MediaKeyEvent::Play,
        1 => MediaKeyEvent::Pause,
        2 => MediaKeyEvent::Toggle,
        3 => MediaKeyEvent::Next,
        4 => MediaKeyEvent::Previous,
        5 => MediaKeyEvent::Stop,
        6 => MediaKeyEvent::Like,
        _ => return,
    };
    glyph_audio::media_keys::push_event(ev);
    gpui_android::wake();
}

/// `PlaybackService.nativeVolume`: the volume keys (or the system's volume
/// panel) while a network device plays; 0..=100.
#[unsafe(no_mangle)]
extern "system" fn Java_app_glyph_PlaybackService_nativeVolume<'local>(_env: EnvUnowned<'local>, _class: JClass<'local>, volume: jint) {
    glyph_audio::media_keys::push_event(MediaKeyEvent::Volume(volume.clamp(0, 100) as u8));
    gpui_android::wake();
}
