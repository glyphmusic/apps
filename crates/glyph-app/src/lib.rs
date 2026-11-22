pub mod account;
pub mod backgrounds;
pub mod commands;
pub mod controller;
pub mod devices;
pub mod discover;
pub mod dispatch;
pub mod dto;
pub mod guard;
pub mod host;
pub mod i18n;
pub mod jam;
pub mod links;
pub mod mcp;
pub mod mcp_tools;
pub mod mpd;
pub mod notify;
pub mod opened;
pub mod page_cache;
pub mod plugins;
pub mod settings;
pub mod storage;
pub mod system;
pub mod theme_icon;
pub mod transfer;
pub mod video;
pub mod wrong_song;

use std::sync::Arc;

pub mod rt {
    use std::future::Future;
    use std::sync::OnceLock;
    use tokio::runtime::Handle;
    use tokio::task::JoinHandle;

    static HANDLE: OnceLock<Handle> = OnceLock::new();

    pub fn set_handle(handle: Handle) {
        let _ = HANDLE.set(handle);
    }

    pub fn handle() -> Handle {
        HANDLE.get().cloned().unwrap_or_else(Handle::current)
    }

    pub fn spawn<F>(fut: F) -> JoinHandle<F::Output>
    where
        F: Future + Send + 'static,
        F::Output: Send + 'static,
    {
        handle().spawn(fut)
    }

    pub fn spawn_blocking<F, R>(f: F) -> JoinHandle<R>
    where
        F: FnOnce() -> R + Send + 'static,
        R: Send + 'static,
    {
        handle().spawn_blocking(f)
    }

    pub fn block_on<F: Future>(fut: F) -> F::Output {
        handle().block_on(fut)
    }
}

/// Initializes shared backend services, timers, account sync, downloads,
/// discovery, transfer worker, plugins, system/updater, and jam/Connect.
pub fn setup(core: &Arc<controller::Core>) {
    {
        let cfg = core.st().config.clone();
        settings::apply_to_player(core, &cfg);
    }
    core.start_backend();
    core.spawn_pump();
    account::setup(core);
    storage::setup(core);
    discover::setup(core);
    transfer::setup(core);
    plugins::setup(core);
    system::setup(core);
    jam::setup(core);
    notify::setup(core);
    let c = core.clone();
    account::on_devices_changed(move || c.emit("devices", jam::devices_json(&c)));
    theme_icon::sync_host_icon(core);
}
