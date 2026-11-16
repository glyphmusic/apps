//! Android WebView shell entry point (`app.glyph.MainActivity` + `PlaybackService`).
//! Runs the shared `glyph-app` controller, Tokio async runtime, `glyph-core`,
//! and `glyph-audio` while delegating UI rendering to Svelte inside WebView.
#![cfg(target_os = "android")]

use glyph_app::controller::Core;
use glyph_app::host::Host;
use glyph_audio::{MediaKeyEvent, MediaKeys, Status};
use glyph_core::dedupe::track_key;
use glyph_protocol::Track;
use jni::errors::LogErrorAndDefault;
use jni::objects::{Global, JClass, JObject, JString};
use jni::sys::{jboolean, jint, jlong};
use jni::{jni_sig, jni_str, Env, EnvUnowned, JValue, JavaVM};
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::Duration;

struct AndroidAppState {
    rt: tokio::runtime::Runtime,
    core: Arc<Core>,
}

static JVM: OnceLock<JavaVM> = OnceLock::new();
static ACTIVITY: Mutex<Option<Global<JObject<'static>>>> = Mutex::new(None);
static STATE: OnceLock<Arc<AndroidAppState>> = OnceLock::new();
static URL_WAITING: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
static LAST_JAM_ACTIVE: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn with_activity<R>(
    label: &str,
    f: impl FnOnce(&mut Env<'_>, &JObject<'_>) -> jni::errors::Result<R>,
) -> Option<R> {
    let vm = JVM.get()?;
    let act_ref = {
        let guard = ACTIVITY.lock().unwrap_or_else(|e| e.into_inner());
        let g = guard.as_ref()?;
        vm.attach_current_thread(|env| env.new_global_ref(g.as_ref()))
            .ok()?
    };

    match vm.attach_current_thread(|env| f(env, act_ref.as_ref())) {
        Ok(val) => Some(val),
        Err(e) => {
            log::warn!("jni `{label}` failed: {e}");
            None
        }
    }
}

fn encode_uri_component(s: &str) -> String {
    let mut enc = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        if b.is_ascii_alphanumeric() || b"-_.!~*'()".contains(&b) {
            enc.push(b as char);
        } else {
            enc.push_str(&format!("%{b:02X}"));
        }
    }
    enc
}

#[derive(Default)]
struct MediaState {
    keys: Option<MediaKeys>,
    rx: Option<flume::Receiver<MediaKeyEvent>>,
    cover_cache: Arc<Mutex<(Option<String>, Option<PathBuf>, bool)>>,
}

struct AndroidHost {
    media: Mutex<MediaState>,
}

impl AndroidHost {
    fn new() -> Self {
        let (keys, rx) = match glyph_app::guard::guard_res(|| MediaKeys::new(None)) {
            Ok(mk) => {
                let rx = mk.events();
                (Some(mk), Some(rx))
            }
            Err(e) => {
                log::warn!("media keys init failed: {e}");
                (None, None)
            }
        };
        Self {
            media: Mutex::new(MediaState {
                keys,
                rx,
                cover_cache: Arc::new(Mutex::new((None, None, false))),
            }),
        }
    }
}

fn toggle_like_current(core: &Arc<Core>) {
    let cur = core.st().current.clone();
    if let Some(t) = cur {
        core.st().library.toggle_like(&t);
        core.library_changed();
    }
}

fn call_complete_invoke(id: i64, ok: bool, payload_json: &str) {
    with_activity("gpuiCompleteInvoke", |env, activity| {
        let json = env.new_string(payload_json)?;
        env.call_method(
            activity,
            jni_str!("gpuiCompleteInvoke"),
            jni_sig!("(JZLjava/lang/String;)V"),
            &[
                JValue::Long(id),
                JValue::Bool(ok.into()),
                JValue::Object(&json),
            ],
        )?;
        Ok(())
    });
}

fn call_emit_event(name: &str, payload_json: &str) {
    with_activity("gpuiEmitEvent", |env, activity| {
        let name = env.new_string(name)?;
        let json = env.new_string(payload_json)?;
        env.call_method(
            activity,
            jni_str!("gpuiEmitEvent"),
            jni_sig!("(Ljava/lang/String;Ljava/lang/String;)V"),
            &[JValue::Object(&name), JValue::Object(&json)],
        )?;
        Ok(())
    });
}

fn take_open_url() -> Option<String> {
    with_activity("gpuiTakeOpenUrl", |env, activity| {
        let value = env
            .call_method(activity, jni_str!("gpuiTakeOpenUrl"), jni_sig!("()Ljava/lang/String;"), &[])?
            .l()?;
        if value.is_null() {
            return Ok(None);
        }
        let s = env.cast_local::<JString>(value)?;
        Ok(Some(s.try_to_string(env)?))
    })
    .flatten()
}

fn fetch_cover_if_needed(core: &Arc<Core>, track: &Track, cache: &Arc<Mutex<(Option<String>, Option<PathBuf>, bool)>>) -> Option<PathBuf> {
    let key = track_key(track);
    let need_fetch = {
        let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
        if c.0.as_deref() == Some(&key) {
            return c.1.clone();
        }
        c.0 = Some(key.clone());
        c.1 = None;
        c.2 = true;
        true
    };
    if need_fetch {
        if let Some(b) = core.backend() {
            let track = track.clone();
            let cache = cache.clone();
            glyph_app::rt::spawn(async move {
                let path = b.cover(&track).await.ok().flatten();
                let mut c = cache.lock().unwrap_or_else(|e| e.into_inner());
                if c.0.as_deref() == Some(&key) {
                    c.1 = path;
                    c.2 = false;
                }
            });
        }
    }
    None
}

impl Host for AndroidHost {
    fn emit(&self, event: &str, payload: Value) {
        let json = serde_json::to_string(&payload).unwrap_or_else(|_| "null".to_string());
        call_emit_event(event, &json);
    }

    fn asset_url(&self, path: &Path) -> String {
        let p = path.to_string_lossy();
        format!("https://glyph.localhost/__asset__?path={}", encode_uri_component(&p))
    }

    fn run_on_main_thread(&self, f: Box<dyn FnOnce() + Send>) {
        f();
    }

    fn on_tick(&self, core: &Arc<Core>) {
        if URL_WAITING.swap(false, std::sync::atomic::Ordering::AcqRel) {
            while let Some(raw) = take_open_url() {
                glyph_app::links::apply(core, &raw);
            }
        }

        let events: Vec<MediaKeyEvent> = {
            let m = self.media.lock().unwrap_or_else(|e| e.into_inner());
            m.rx.as_ref().map(|rx| rx.try_iter().collect()).unwrap_or_default()
        };
        for ev in events {
            match ev {
                MediaKeyEvent::Play => {
                    if core.player_dto().status != "playing" {
                        core.toggle_play();
                    }
                }
                MediaKeyEvent::Pause => {
                    if core.player_dto().status == "playing" {
                        core.toggle_play();
                    }
                }
                MediaKeyEvent::Toggle => core.toggle_play(),
                MediaKeyEvent::Next => core.next(false),
                MediaKeyEvent::Previous => core.previous(),
                MediaKeyEvent::Stop => {
                    if let Some(p) = &core.player {
                        p.stop();
                    }
                }
                MediaKeyEvent::SeekForward => core.seek(core.player_dto().position + 5.0),
                MediaKeyEvent::SeekBackward => core.seek((core.player_dto().position - 5.0).max(0.0)),
                MediaKeyEvent::Like => toggle_like_current(core),
                MediaKeyEvent::Volume(v) => core.set_volume(f32::from(v) / 100.0),
            }
        }

        let (current, liked, playing, buffering, position, remote_volume) = {
            let st = core.st();
            let ps = core.player_state();
            let current = st.current.clone().or_else(|| ps.as_ref().and_then(|s| s.track.clone()));
            let liked = current.as_ref().is_some_and(|c| st.library.is_liked(c));
            let hold = glyph_app::jam::hold_play();
            let loading = ps.as_ref().is_some_and(|s| s.status == Status::Loading);
            let playing = (ps.as_ref().is_some_and(|s| s.status == Status::Playing) && !st.resolving)
                || (st.resolving && !hold);
            let buffering = (st.resolving || loading) && !hold;
            let pos = ps.as_ref().map_or(Duration::ZERO, |s| s.position);
            let remote_vol = if glyph_app::jam::remote_on() {
                ps.as_ref().map(|s| (s.volume.clamp(0.0, 1.0) * 100.0).round() as u8)
            } else {
                None
            };
            (current, liked, playing, buffering, pos, remote_vol)
        };

        let jam_active = glyph_app::jam::jam_active();
        if LAST_JAM_ACTIVE.swap(jam_active, std::sync::atomic::Ordering::AcqRel) != jam_active {
            with_activity("jam active", |env, activity| {
                env.call_method(activity, jni_str!("gpuiJamActive"), jni_sig!("(Z)V"), &[JValue::Bool(jam_active.into())])?;
                Ok(())
            });
        }

        let m = self.media.lock().unwrap_or_else(|e| e.into_inner());
        let cover = current.as_ref().and_then(|t| fetch_cover_if_needed(core, t, &m.cover_cache));
        if let Some(mk) = &m.keys {
            mk.set_playback_state(current.as_ref(), playing, buffering, position);
            mk.set_artwork(cover.as_deref(), liked);
            mk.set_remote_volume(remote_volume);
        }
    }

    fn open_url(&self, url: &str) -> Result<(), String> {
        let called = with_activity("gpuiOpenUrl", |env, activity| {
            let u = env.new_string(url)?;
            env.call_method(activity, jni_str!("gpuiOpenUrl"), jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&u)])?;
            Ok(())
        });
        if called.is_some() {
            Ok(())
        } else {
            Err("activity unavailable".into())
        }
    }

    fn write_clipboard(&self, text: &str) -> Result<(), String> {
        let called = with_activity("gpuiSetClipboard", |env, activity| {
            let s = env.new_string(text)?;
            env.call_method(activity, jni_str!("gpuiSetClipboard"), jni_sig!("(Ljava/lang/String;)V"), &[JValue::Object(&s)])?;
            Ok(())
        });
        if called.is_some() {
            Ok(())
        } else {
            Err("activity unavailable".into())
        }
    }

    fn set_theme_icon(&self, theme_id: &str, accent: u32) {
        with_activity("gpuiSetThemeIcon", |env, activity| {
            let t = env.new_string(theme_id)?;
            env.call_method(
                activity,
                jni_str!("gpuiSetThemeIcon"),
                jni_sig!("(Ljava/lang/String;I)V"),
                &[JValue::Object(&t), JValue::Int(accent as i32)],
            )?;
            Ok(())
        });
    }
}

fn install_platform_hooks() {
    glyph_audio::media_keys::set_now_playing_sink(Box::new(|now: &glyph_audio::media_keys::NowPlaying<'_>| {
        let (title, artist, dur) = match now.track {
            Some(t) => (t.title.clone(), t.artist.clone(), t.duration_ms.unwrap_or(0)),
            None => (String::new(), String::new(), 0),
        };
        let cover = now.cover.map(|p| p.to_string_lossy().into_owned()).unwrap_or_default();
        let remote_volume = now.remote_volume.map_or(-1, i32::from);
        with_activity("now playing", |env, activity| {
            let title = env.new_string(&title)?;
            let artist = env.new_string(&artist)?;
            let cover = env.new_string(&cover)?;
            env.call_method(
                activity,
                jni_str!("gpuiNowPlaying"),
                jni_sig!("(Ljava/lang/String;Ljava/lang/String;ZZJJLjava/lang/String;ZI)V"),
                &[
                    JValue::Object(&title),
                    JValue::Object(&artist),
                    JValue::Bool(now.playing.into()),
                    JValue::Bool(now.buffering.into()),
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

    glyph_core::updater::set_apk_installer(Box::new(|path| {
        let path = path.to_string_lossy().into_owned();
        with_activity("install update", |env, activity| {
            let path = env.new_string(&path)?;
            env.call_method(
                activity,
                jni_str!("gpuiInstallApk"),
                jni_sig!("(Ljava/lang/String;)V"),
                &[JValue::Object(&path)],
            )?;
            Ok(())
        })
        .is_some()
    }));

    std::thread::spawn(|| {
        for _ in 0..100 {
            let id = with_activity("android id", |env, activity| {
                let value = env
                    .call_method(activity, jni_str!("gpuiAndroidId"), jni_sig!("()Ljava/lang/String;"), &[])?
                    .l()?;
                if value.is_null() {
                    return Ok(None);
                }
                let value = env.cast_local::<JString>(value)?;
                Ok(Some(value.try_to_string(env)?))
            });
            match id {
                Some(Some(id)) => {
                    glyph_core::hardware::set_platform_id(&id);
                    return;
                }
                Some(None) => return,
                None => std::thread::sleep(Duration::from_millis(100)),
            }
        }
    });
}

fn configure_dirs(files_dir: &str) {
    let base = PathBuf::from(files_dir);
    let set = |key: &str, dir: PathBuf| {
        let _ = std::fs::create_dir_all(&dir);
        std::env::set_var(key, dir);
    };
    set("GLYPH_DATA_DIR", base.join("data"));
    set("GLYPH_CACHE_DIR", base.join("cache"));
    let config = base.join("config");
    let _ = std::fs::create_dir_all(&config);
    std::env::set_var("GLYPH_CONFIG", config.join("config.toml"));
}

/// Called from `MainActivity.onCreate` once WebView is initialized.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_glyph_MainActivity_nativeInit<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    activity: JObject<'local>,
    files_dir: JString<'local>,
) {
    android_logger::init_once(
        android_logger::Config::default()
            .with_max_level(log::LevelFilter::Info)
            .with_tag("glyph"),
    );

    let init_info = env
        .with_env(|env| -> jni::errors::Result<Option<(JavaVM, Global<JObject<'static>>, Global<JObject<'static>>, String)>> {
            let vm = env.get_java_vm()?;
            let global = env.new_global_ref(&activity)?;
            let app_ctx = env
                .call_method(&activity, jni_str!("getApplicationContext"), jni_sig!("()Landroid/content/Context;"), &[])?
                .l()?;
            let global_app = env.new_global_ref(&app_ctx)?;
            let dir = files_dir.try_to_string(env)?;
            Ok(Some((vm, global, global_app, dir)))
        })
        .resolve::<LogErrorAndDefault>();

    let Some((vm, global_act, global_app, files_dir)) = init_info else {
        return;
    };
    let _ = JVM.set(vm.clone());
    *ACTIVITY.lock().unwrap_or_else(|e| e.into_inner()) = Some(global_act);

    static INIT_NDK: std::sync::Once = std::sync::Once::new();
    INIT_NDK.call_once(|| unsafe {
        ndk_context::initialize_android_context(
            vm.get_raw().cast(),
            global_app.as_raw().cast(),
        );
        std::mem::forget(global_app);
    });

    STATE.get_or_init(|| {
        std::panic::set_hook(Box::new(|info| log::error!("panic: {info}")));
        configure_dirs(&files_dir);
        install_platform_hooks();

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .expect("tokio runtime");
        glyph_app::rt::set_handle(rt.handle().clone());

        let host = Arc::new(AndroidHost::new());
        let core = Core::new(host);
        glyph_app::setup(&core);

        while let Some(raw) = take_open_url() {
            glyph_app::links::apply(&core, &raw);
        }

        Arc::new(AndroidAppState { rt, core })
    });
}

/// Asynchronous command bridge called from WebView `window.__GLYPH_ANDROID__.invoke`.
#[unsafe(no_mangle)]
pub extern "system" fn Java_app_glyph_MainActivity_nativeInvoke<'local>(
    mut env: EnvUnowned<'local>,
    _this: JObject<'local>,
    id: jlong,
    cmd: JString<'local>,
    args_json: JString<'local>,
) {
    let parsed = env
        .with_env(|env| -> jni::errors::Result<(String, String)> {
            let cmd = cmd.try_to_string(env)?;
            let args = args_json.try_to_string(env)?;
            Ok((cmd, args))
        })
        .resolve::<LogErrorAndDefault>();
    let (cmd, args_json) = parsed;
    if cmd.is_empty() {
        return;
    }

    let Some(state) = STATE.get().cloned() else {
        let err_json = serde_json::to_string("app not initialized").unwrap_or_else(|_| "\"init error\"".into());
        call_complete_invoke(id as i64, false, &err_json);
        return;
    };

    let args: Value = if args_json.trim().is_empty() {
        Value::Object(Default::default())
    } else {
        serde_json::from_str(&args_json).unwrap_or_else(|_| Value::Object(Default::default()))
    };

    let core = Arc::clone(&state.core);
    state.rt.spawn(async move {
        use futures::FutureExt;
        let panic_res = std::panic::AssertUnwindSafe(async {
            glyph_app::dispatch::dispatch(&core, &cmd, args).await
        })
        .catch_unwind()
        .await;

        let (ok, json) = match panic_res {
            Ok(Ok(v)) => (true, serde_json::to_string(&v).unwrap_or_else(|_| "null".into())),
            Ok(Err(e)) => (false, serde_json::to_string(&e).unwrap_or_else(|_| "\"error\"".into())),
            Err(panic_err) => {
                let msg = if let Some(s) = panic_err.downcast_ref::<&str>() {
                    s.to_string()
                } else if let Some(s) = panic_err.downcast_ref::<String>() {
                    s.clone()
                } else {
                    format!("panic in command `{cmd}`")
                };
                log::error!("command `{cmd}` panicked: {msg}");
                (false, serde_json::to_string(&msg).unwrap_or_else(|_| "\"panic\"".into()))
            }
        };
        call_complete_invoke(id as i64, ok, &json);
    });
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_glyph_MainActivity_nativeConnectivity(
    _env: EnvUnowned,
    _this: JObject,
    online: jboolean,
) {
    glyph_core::connectivity::set_platform_online(online);
    if online {
        glyph_core::outage::clear();
        if let Some(st) = STATE.get() {
            if let Some(b) = st.core.backend() {
                b.clear_cooldowns();
            }
        }
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_glyph_MainActivity_nativeInsets<'local>(
    _env: EnvUnowned<'local>,
    _this: JObject<'local>,
    _top: jint,
    _right: jint,
    _bottom: jint,
    _left: jint,
    _ime_top: jint,
    _ime_right: jint,
    _ime_bottom: jint,
    _ime_left: jint,
) {
    // WebView applies system-bar and IME CSS custom properties (--safe-* / --ime-bottom).
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_glyph_MainActivity_nativeUrlWaiting<'local>(
    _env: EnvUnowned<'local>,
    _this: JObject<'local>,
) {
    if let Some(state) = STATE.get().cloned() {
        while let Some(raw) = take_open_url() {
            glyph_app::links::apply(&state.core, &raw);
        }
    } else {
        URL_WAITING.store(true, std::sync::atomic::Ordering::Release);
    }
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_glyph_PlaybackService_nativeMediaKey<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    code: jint,
) {
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
}

#[unsafe(no_mangle)]
pub extern "system" fn Java_app_glyph_PlaybackService_nativeVolume<'local>(
    _env: EnvUnowned<'local>,
    _class: JClass<'local>,
    volume: jint,
) {
    glyph_audio::media_keys::push_event(MediaKeyEvent::Volume(volume.clamp(0, 100) as u8));
}
