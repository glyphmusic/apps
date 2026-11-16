//! Enforces a fixed 500 MB RAM budget across the desktop process tree
//! (`glyph.exe` and all child `msedgewebview2.exe` processes).
//!
//! Combines three complementary layers:
//! 1. Chromium/WebView2 startup caps (`--js-flags=--max-old-space-size=128`,
//!    process/cache limits) set before Tauri initializes WebView2.
//! 2. Proactive in-app + WebView2 cache shedding (`Core::trim_memory` and
//!    `ICoreWebView2_19::SetMemoryUsageTargetLevel(LOW)`) above soft thresholds.
//! 3. Windows kernel working-set trimming (`EmptyWorkingSet`) and proportional
//!    per-process `QUOTA_LIMITS_HARDWS_MAX_ENABLE` working-set limits so the
//!    tree's physical private RAM stays strictly under 500 MB without failing
//!    virtual allocations (`malloc`/V8 OOM crashes).

use glyph_app::controller::Core;
use std::sync::Arc;

/// Fixed 500 MB RAM budget (binary MiB displayed by Windows Task Manager).
pub const RAM_CAP_BYTES: usize = 500 * 1024 * 1024;
/// Active physical ceiling (~487.6 MB decimal, 465 MiB Task Manager) so both
/// MiB and decimal MB stay safely under 500 MB.
const ENFORCE_CAP_BYTES: usize = 465 * 1024 * 1024;
const HARD_TRIM_BYTES: usize = 400 * 1024 * 1024;
const SOFT_TRIM_BYTES: usize = 330 * 1024 * 1024;
const RELEASE_HYSTERESIS_BYTES: usize = 290 * 1024 * 1024;

const DEFAULT_BROWSER_ARGS: &[(&str, &str)] = &[
    ("--js-flags=", "--js-flags=--max-old-space-size=128"),
    ("--renderer-process-limit=", "--renderer-process-limit=2"),
    ("--disk-cache-size=", "--disk-cache-size=33554432"),
    ("--media-cache-size=", "--media-cache-size=16777216"),
];

/// Merges WebView2 memory/cache flags into an existing `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS`
/// value without duplicating any flag the caller already configured.
fn merge_browser_args(existing: &str) -> String {
    let mut parts: Vec<String> = existing.split_whitespace().map(str::to_string).collect();
    for (prefix, flag) in DEFAULT_BROWSER_ARGS {
        let bare = prefix.trim_end_matches('=');
        if !parts.iter().any(|p| p.starts_with(prefix) || p == bare) {
            parts.push((*flag).to_string());
        }
    }
    parts.join(" ")
}

/// Applies Chromium V8 heap and disk/media cache bounds before WebView2 boots.
pub fn configure_browser_env() {
    #[cfg(windows)]
    {
        let existing = std::env::var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS").unwrap_or_default();
        let merged = merge_browser_args(&existing);
        if merged != existing {
            // SAFETY: called once at the start of `main()` before worker threads
            // or Tauri/WebView2 initialization read the environment.
            unsafe { std::env::set_var("WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS", merged) };
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ProcMem {
    pid: u32,
    working_set: usize,
    private_ws: usize,
    commit: usize,
}

impl ProcMem {
    fn shared_ws(self) -> usize {
        self.working_set.saturating_sub(self.private_ws)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Pressure {
    Low,
    Soft,
    HardTrim,
    EnforceCap,
}

fn classify_pressure(total_priv: usize, total_commit: usize, was_low_target: bool) -> Pressure {
    if total_priv >= ENFORCE_CAP_BYTES {
        Pressure::EnforceCap
    } else if total_priv >= HARD_TRIM_BYTES {
        Pressure::HardTrim
    } else if total_priv >= SOFT_TRIM_BYTES || total_commit >= ENFORCE_CAP_BYTES {
        Pressure::Soft
    } else if was_low_target && total_priv > RELEASE_HYSTERESIS_BYTES {
        Pressure::Soft
    } else {
        Pressure::Low
    }
}

/// Computes per-process hard `WorkingSetSize` limits (`(pid, max_ws_bytes)`) so
/// the sum of private working sets across `procs` fits within `cap_private`
/// while preserving each process's shared-DLL working-set delta.
fn compute_working_set_caps(procs: &[ProcMem], cap_private: usize) -> Vec<(u32, usize)> {
    const MIN_FLOOR: usize = 16 * 1024 * 1024;
    let total_priv: usize = procs.iter().map(|p| p.private_ws).sum();
    if total_priv <= cap_private || procs.is_empty() {
        return Vec::new();
    }

    let floors: Vec<usize> = procs.iter().map(|p| p.private_ws.min(MIN_FLOOR)).collect();
    let floor_sum: usize = floors.iter().sum();

    if floor_sum >= cap_private {
        let mut out = Vec::with_capacity(procs.len());
        for p in procs {
            let priv_cap = ((p.private_ws as u128 * cap_private as u128) / (total_priv.max(1) as u128)) as usize;
            out.push((p.pid, priv_cap.saturating_add(p.shared_ws()).max(MIN_FLOOR)));
        }
        return out;
    }

    let headroom = cap_private - floor_sum;
    let excess: Vec<usize> = procs.iter().zip(&floors).map(|(p, &f)| p.private_ws.saturating_sub(f)).collect();
    let excess_sum: usize = excess.iter().sum();

    let mut out = Vec::with_capacity(procs.len());
    for ((p, &f), &ex) in procs.iter().zip(&floors).zip(&excess) {
        if ex == 0 {
            continue;
        }
        let share = ((ex as u128 * headroom as u128) / (excess_sum.max(1) as u128)) as usize;
        let target_priv = f + share;
        if p.private_ws > target_priv {
            out.push((p.pid, target_priv.saturating_add(p.shared_ws()).max(MIN_FLOOR)));
        }
    }
    out
}

use std::time::{Duration, Instant};

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct DormantController {
    pub is_dormant: bool,
    pub candidate_since: Option<Instant>,
    pub last_dormant_trim: Option<Instant>,
}

impl DormantController {
    #[allow(dead_code)]
    pub fn new() -> Self {
        Self::default()
    }

    /// Evaluates window state transition.
    /// Returns true if dormant state was immediately cancelled.
    pub fn on_window_state_change(&mut self, is_hidden_or_minimized: bool, now: Instant) -> bool {
        if is_hidden_or_minimized {
            if !self.is_dormant && self.candidate_since.is_none() {
                self.candidate_since = Some(now);
            }
            false
        } else {
            self.candidate_since = None;
            if self.is_dormant {
                self.is_dormant = false;
                true
            } else {
                false
            }
        }
    }

    /// Evaluates if debounce threshold (1.5s) has elapsed and enters dormancy.
    /// Returns true if transitioned from active to dormant.
    pub fn check_enter_dormant(&mut self, is_hidden_or_minimized: bool, now: Instant, debounce: Duration) -> bool {
        if !is_hidden_or_minimized {
            self.candidate_since = None;
            return false;
        }
        if self.is_dormant {
            return false;
        }
        if let Some(candidate) = self.candidate_since {
            if now.duration_since(candidate) >= debounce {
                self.is_dormant = true;
                self.candidate_since = None;
                self.last_dormant_trim = Some(now);
                return true;
            }
        } else {
            self.candidate_since = Some(now);
        }
        false
    }

    /// Returns true if a periodic dormant trim (every 20s) is due.
    pub fn should_periodic_trim(&mut self, now: Instant, interval: Duration) -> bool {
        if !self.is_dormant {
            return false;
        }
        match self.last_dormant_trim {
            Some(last) if now.duration_since(last) >= interval => {
                self.last_dormant_trim = Some(now);
                true
            }
            None => {
                self.last_dormant_trim = Some(now);
                true
            }
            _ => false,
        }
    }

    /// Determines QoS throttle state for a process within the tree.
    /// Child processes are always throttled when dormant.
    /// Root process is throttled ONLY when dormant AND audio is NOT playing.
    pub fn should_throttle_process(is_dormant: bool, is_root: bool, audio_playing: bool) -> bool {
        if !is_dormant {
            return false;
        }
        if is_root {
            !audio_playing
        } else {
            true
        }
    }
}

/// Notifies the memory guard of window resize, minimize, focus, or visibility changes.
pub fn notify_window_state(app: &tauri::AppHandle) {
    #[cfg(windows)]
    win::handle_window_state_change(app);
    #[cfg(not(windows))]
    let _ = app;
}

/// Starts the background 500 MB RAM cap monitor and enforcer for the desktop app.
pub fn start(app: tauri::AppHandle, core: Arc<Core>) {
    #[cfg(windows)]
    {
        std::thread::Builder::new()
            .name("glyph-ram-cap".into())
            .spawn(move || win::monitor_loop(app, core))
            .ok();
    }
    #[cfg(not(windows))]
    let _ = (app, core);
}

#[cfg(windows)]
mod win {
    use super::*;
    use std::collections::HashSet;
    use std::sync::{Condvar, Mutex};
    use std::time::{Duration, Instant};
    use tauri::Manager;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::Diagnostics::ToolHelp::{
        CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS,
    };
    use windows::Win32::System::Memory::{
        QUOTA_LIMITS_HARDWS_MAX_DISABLE, QUOTA_LIMITS_HARDWS_MAX_ENABLE, SetProcessWorkingSetSizeEx,
    };
    use windows::Win32::System::ProcessStatus::{
        EmptyWorkingSet, GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS, PROCESS_MEMORY_COUNTERS_EX,
        PROCESS_MEMORY_COUNTERS_EX2,
    };
    use windows::Win32::System::Threading::{
        GetCurrentProcessId, OpenProcess, PROCESS_QUERY_INFORMATION, PROCESS_QUERY_LIMITED_INFORMATION,
        PROCESS_SET_QUOTA,
    };

    static DORMANT: Mutex<DormantController> = Mutex::new(DormantController {
        is_dormant: false,
        candidate_since: None,
        last_dormant_trim: None,
    });
    static DORMANT_NOTIFY: Condvar = Condvar::new();

    pub(super) fn handle_window_state_change(app: &tauri::AppHandle) {
        let Some(win) = app.get_webview_window("main") else { return };
        let is_hidden_or_minimized = win.is_minimized().unwrap_or(false) || !win.is_visible().unwrap_or(true);
        let exited_dormant = {
            let mut ctrl = DORMANT.lock().unwrap();
            ctrl.on_window_state_change(is_hidden_or_minimized, Instant::now())
        };
        if exited_dormant {
            let root_pid = unsafe { GetCurrentProcessId() };
            crate::playback_qos::set_process_efficiency_mode(root_pid, false);
            for pid in collect_tree_pids(root_pid) {
                if pid != root_pid {
                    crate::playback_qos::set_process_efficiency_mode(pid, false);
                }
            }
            crate::desktop::resume_main_webview(app);
            crate::desktop::set_webview_low_memory(app, false);
            tracing::debug!("memory_guard: exited dormant mode (WebView resumed, EcoQoS disabled)");
        }
        DORMANT_NOTIFY.notify_all();
    }

    pub(super) fn is_audio_playing(core: &Core) -> bool {
        let ps = core.player_state();
        let st = core.st();
        (ps.as_ref().is_some_and(|s| {
            s.status == glyph_audio::Status::Playing || s.status == glyph_audio::Status::Loading
        }) && !st.resolving)
            || (st.resolving && !glyph_app::jam::hold_play())
    }

    pub(super) struct OwnedHandle(HANDLE);
    impl Drop for OwnedHandle {
        fn drop(&mut self) {
            // SAFETY: `self.0` was returned by a successful Win32 handle call.
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }

    pub(super) fn collect_tree_pids(root_pid: u32) -> Vec<u32> {
        let mut pairs = Vec::new();
        // SAFETY: `CreateToolhelp32Snapshot` with `TH32CS_SNAPPROCESS` reads a kernel snapshot.
        let Ok(snap) = (unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }) else {
            return vec![root_pid];
        };
        let _guard = OwnedHandle(snap);
        let mut entry = PROCESSENTRY32W {
            dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
            ..Default::default()
        };
        // SAFETY: `entry.dwSize` matches `PROCESSENTRY32W` and `snap` is valid.
        if unsafe { Process32FirstW(snap, &mut entry) }.is_ok() {
            loop {
                pairs.push((entry.th32ProcessID, entry.th32ParentProcessID));
                if unsafe { Process32NextW(snap, &mut entry) }.is_err() {
                    break;
                }
            }
        }
        let mut seen = HashSet::new();
        seen.insert(root_pid);
        let mut queue = vec![root_pid];
        let mut i = 0;
        while i < queue.len() {
            let parent = queue[i];
            i += 1;
            for &(pid, ppid) in &pairs {
                if ppid == parent && seen.insert(pid) {
                    queue.push(pid);
                }
            }
        }
        queue
    }

    fn open_proc(pid: u32) -> Option<(OwnedHandle, bool)> {
        // SAFETY: `OpenProcess` opens a handle cleaned up by `OwnedHandle`.
        if let Ok(h) = unsafe { OpenProcess(PROCESS_QUERY_INFORMATION | PROCESS_SET_QUOTA, false, pid) } {
            return Some((OwnedHandle(h), true));
        }
        if let Ok(h) = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_SET_QUOTA, false, pid) } {
            return Some((OwnedHandle(h), true));
        }
        // SAFETY: fallback to read-only query rights if quota rights are denied.
        let h = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) }.ok()?;
        Some((OwnedHandle(h), false))
    }

    pub(super) fn read_proc_mem(pid: u32) -> Option<(OwnedHandle, bool, ProcMem)> {
        let (h, can_quota) = open_proc(pid)?;
        let mut ex2 = PROCESS_MEMORY_COUNTERS_EX2 {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX2>() as u32,
            ..Default::default()
        };
        // SAFETY: `PROCESS_MEMORY_COUNTERS_EX2` begins with `PROCESS_MEMORY_COUNTERS` and `cb` matches its exact size.
        if unsafe {
            GetProcessMemoryInfo(
                h.0,
                (&mut ex2 as *mut PROCESS_MEMORY_COUNTERS_EX2).cast::<PROCESS_MEMORY_COUNTERS>(),
                ex2.cb,
            )
        }
        .is_ok()
        {
            return Some((
                h,
                can_quota,
                ProcMem {
                    pid,
                    working_set: ex2.WorkingSetSize,
                    private_ws: ex2.PrivateWorkingSetSize.min(ex2.WorkingSetSize),
                    commit: ex2.PrivateUsage,
                },
            ));
        }

        let mut ex = PROCESS_MEMORY_COUNTERS_EX {
            cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS_EX>() as u32,
            ..Default::default()
        };
        // SAFETY: fallback to `PROCESS_MEMORY_COUNTERS_EX` on older Windows versions without EX2.
        unsafe {
            GetProcessMemoryInfo(
                h.0,
                (&mut ex as *mut PROCESS_MEMORY_COUNTERS_EX).cast::<PROCESS_MEMORY_COUNTERS>(),
                ex.cb,
            )
        }
        .ok()?;
        Some((
            h,
            can_quota,
            ProcMem {
                pid,
                working_set: ex.WorkingSetSize,
                private_ws: ex.PrivateUsage.min(ex.WorkingSetSize),
                commit: ex.PrivateUsage,
            },
        ))
    }

    fn clear_hard_ws(procs: &[(OwnedHandle, bool, ProcMem)]) {
        for (h, can_quota, _) in procs {
            if *can_quota {
                // SAFETY: `(-1, -1)` with `QUOTA_LIMITS_HARDWS_MAX_DISABLE` restores default kernel WS limits.
                unsafe {
                    let _ = SetProcessWorkingSetSizeEx(h.0, usize::MAX, usize::MAX, QUOTA_LIMITS_HARDWS_MAX_DISABLE);
                }
            }
        }
    }

    pub(super) fn monitor_loop(app: tauri::AppHandle, core: Arc<Core>) {
        // Let initial WebView2 startup settle before applying memory pressure checks.
        std::thread::sleep(Duration::from_secs(4));
        let root_pid = unsafe { GetCurrentProcessId() };
        let mut low_target = false;
        let mut hard_pinned = false;
        let mut last_soft_trim = Instant::now() - Duration::from_secs(60);
        let mut last_hard_trim = Instant::now() - Duration::from_secs(60);

        loop {
            let pids = collect_tree_pids(root_pid);
            let entries: Vec<(OwnedHandle, bool, ProcMem)> = pids.iter().copied().filter_map(read_proc_mem).collect();
            let stats: Vec<ProcMem> = entries.iter().map(|e| e.2).collect();
            let total_priv: usize = stats.iter().map(|s| s.private_ws).sum();
            let total_commit: usize = stats.iter().map(|s| s.commit).sum();

            let is_hidden_or_minimized = app.get_webview_window("main").map_or(false, |w| {
                w.is_minimized().unwrap_or(false) || !w.is_visible().unwrap_or(true)
            });
            let now = Instant::now();
            let (just_entered_dormant, is_dormant, do_dormant_trim) = {
                let mut ctrl = DORMANT.lock().unwrap();
                let entered = ctrl.check_enter_dormant(is_hidden_or_minimized, now, Duration::from_millis(1500));
                let periodic_trim = ctrl.should_periodic_trim(now, Duration::from_secs(20));
                (entered, ctrl.is_dormant, periodic_trim)
            };

            let audio_playing = is_audio_playing(&core);

            if just_entered_dormant {
                crate::desktop::set_webview_low_memory(&app, true);
                crate::desktop::suspend_main_webview(&app);
                for &pid in &pids {
                    let throttle = DormantController::should_throttle_process(true, pid == root_pid, audio_playing);
                    crate::playback_qos::set_process_efficiency_mode(pid, throttle);
                }
                core.trim_memory(true);
                for (h, can_quota, _) in &entries {
                    if *can_quota {
                        unsafe { let _ = EmptyWorkingSet(h.0); }
                    }
                }
                tracing::debug!(audio_playing, "memory_guard: entered dormant mode (WebView suspended, EcoQoS enabled)");
            } else if is_dormant {
                if !is_hidden_or_minimized {
                    handle_window_state_change(&app);
                    continue;
                }
                let root_throttle = DormantController::should_throttle_process(true, true, audio_playing);
                crate::playback_qos::set_process_efficiency_mode(root_pid, root_throttle);
                for &pid in &pids {
                    if pid != root_pid {
                        crate::playback_qos::set_process_efficiency_mode(pid, true);
                    }
                }

                if do_dormant_trim {
                    core.trim_memory(true);
                    for (h, can_quota, m) in &entries {
                        if *can_quota && (!audio_playing || m.pid != root_pid) {
                            unsafe { let _ = EmptyWorkingSet(h.0); }
                        }
                    }
                    tracing::debug!("memory_guard: dormant periodic 20s memory trim");
                }
            } else {
                let pressure = classify_pressure(total_priv, total_commit, low_target);
                match pressure {
                    Pressure::Low => {
                        if low_target {
                            low_target = false;
                            crate::desktop::set_webview_low_memory(&app, false);
                        }
                        if hard_pinned {
                            hard_pinned = false;
                            clear_hard_ws(&entries);
                        }
                    }
                    Pressure::Soft => {
                        if !low_target {
                            low_target = true;
                            crate::desktop::set_webview_low_memory(&app, true);
                        }
                        if last_soft_trim.elapsed() >= Duration::from_secs(8) {
                            last_soft_trim = Instant::now();
                            core.trim_memory(false);
                        }
                    }
                    Pressure::HardTrim | Pressure::EnforceCap => {
                        let enforce = pressure == Pressure::EnforceCap;
                        if !low_target {
                            low_target = true;
                            crate::desktop::set_webview_low_memory(&app, true);
                        }
                        if last_hard_trim.elapsed() >= Duration::from_secs(if enforce { 2 } else { 5 }) {
                            last_hard_trim = Instant::now();
                            core.trim_memory(true);
                            for (h, can_quota, m) in &entries {
                                if *can_quota && m.private_ws > 24 * 1024 * 1024 {
                                    // SAFETY: moves excess clean/reclaimable pages from process working set.
                                    unsafe { let _ = EmptyWorkingSet(h.0); }
                                }
                            }
                            if enforce {
                                let caps = compute_working_set_caps(&stats, ENFORCE_CAP_BYTES);
                                for (pid, max_ws) in caps {
                                    if let Some((h, true, _)) = entries.iter().find(|e| e.2.pid == pid) {
                                        let min_ws = (4 * 1024 * 1024).min(max_ws / 2);
                                        // SAFETY: hard-caps process physical working set below its share of the 500 MB budget.
                                        let res = unsafe {
                                            SetProcessWorkingSetSizeEx(
                                                h.0,
                                                min_ws,
                                                max_ws,
                                                QUOTA_LIMITS_HARDWS_MAX_ENABLE,
                                            )
                                        };
                                        if res.is_ok() {
                                            hard_pinned = true;
                                        }
                                    }
                                }
                                tracing::info!(
                                    priv_mb = total_priv / (1024 * 1024),
                                    commit_mb = total_commit / (1024 * 1024),
                                    cap_mb = RAM_CAP_BYTES / (1024 * 1024),
                                    "memory_guard: enforced 500 MB working-set cap"
                                );
                            } else {
                                tracing::debug!(
                                    priv_mb = total_priv / (1024 * 1024),
                                    commit_mb = total_commit / (1024 * 1024),
                                    "memory_guard: trimmed process working sets"
                                );
                            }
                        }
                    }
                }
            }

            let pressure = classify_pressure(total_priv, total_commit, low_target);
            let sleep_ms = if is_dormant {
                500
            } else if matches!(pressure, Pressure::HardTrim | Pressure::EnforceCap) {
                1500
            } else {
                2500
            };
            let guard = DORMANT.lock().unwrap();
            let _ = DORMANT_NOTIFY.wait_timeout(guard, Duration::from_millis(sleep_ms)).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_browser_args_adds_missing_flags_and_preserves_existing() {
        let empty = merge_browser_args("");
        assert!(empty.contains("--js-flags=--max-old-space-size=128"));
        assert!(empty.contains("--renderer-process-limit=2"));
        assert!(empty.contains("--disk-cache-size=33554432"));
        assert!(empty.contains("--media-cache-size=16777216"));

        let custom = merge_browser_args("--js-flags=--max-old-space-size=96 --foo=bar");
        assert!(custom.contains("--js-flags=--max-old-space-size=96"));
        assert!(!custom.contains("--max-old-space-size=128"));
        assert!(custom.contains("--foo=bar"));
        assert!(custom.contains("--renderer-process-limit=2"));
    }

    #[test]
    fn working_set_caps_enforce_total_private_budget() {
        let m = 1024 * 1024;
        let procs = vec![
            ProcMem { pid: 1, working_set: 60 * m, private_ws: 20 * m, commit: 25 * m },
            ProcMem { pid: 2, working_set: 390 * m, private_ws: 320 * m, commit: 360 * m },
            ProcMem { pid: 3, working_set: 260 * m, private_ws: 220 * m, commit: 240 * m },
        ];
        let cap = ENFORCE_CAP_BYTES;
        let caps = compute_working_set_caps(&procs, cap);
        assert!(!caps.is_empty());
        let resulting_priv: usize = procs
            .iter()
            .map(|p| {
                caps.iter()
                    .find(|(pid, _)| *pid == p.pid)
                    .map(|(_, max_ws)| max_ws.saturating_sub(p.shared_ws()))
                    .unwrap_or(p.private_ws)
            })
            .sum();
        assert!(resulting_priv <= cap + procs.len(), "resulting private sum {resulting_priv} exceeded cap {cap}");
    }

    #[test]
    fn pressure_classification_follows_thresholds_and_hysteresis() {
        let m = 1024 * 1024;
        assert_eq!(classify_pressure(200 * m, 250 * m, false), Pressure::Low);
        assert_eq!(classify_pressure(300 * m, 320 * m, true), Pressure::Soft);
        assert_eq!(classify_pressure(280 * m, 320 * m, true), Pressure::Low);
        assert_eq!(classify_pressure(350 * m, 360 * m, false), Pressure::Soft);
        assert_eq!(classify_pressure(410 * m, 420 * m, true), Pressure::HardTrim);
        assert_eq!(classify_pressure(470 * m, 480 * m, true), Pressure::EnforceCap);
    }

    #[cfg(windows)]
    #[test]
    fn windows_process_tree_and_memory_query_succeeds() {
        let pid = unsafe { windows::Win32::System::Threading::GetCurrentProcessId() };
        let pids = win::collect_tree_pids(pid);
        assert!(pids.contains(&pid));
        let (_h, _quota, mem) = win::read_proc_mem(pid).expect("read_proc_mem on self");
        assert!(mem.working_set > 0);
        assert!(mem.private_ws > 0);
    }

    #[test]
    fn dormant_controller_debounces_and_cancels_immediately() {
        let mut ctrl = DormantController::new();
        let t0 = Instant::now();
        let debounce = Duration::from_millis(1500);

        // Window minimized: candidate marked, not yet dormant
        assert!(!ctrl.on_window_state_change(true, t0));
        assert!(!ctrl.check_enter_dormant(true, t0 + Duration::from_millis(500), debounce));
        assert!(!ctrl.is_dormant);

        // Alt-tab back before 1.5s: cancelled, not dormant
        assert!(!ctrl.on_window_state_change(false, t0 + Duration::from_millis(600)));
        assert_eq!(ctrl.candidate_since, None);
        assert!(!ctrl.is_dormant);

        // Minimized again:
        let t1 = t0 + Duration::from_millis(1000);
        assert!(!ctrl.on_window_state_change(true, t1));
        assert!(!ctrl.check_enter_dormant(true, t1 + Duration::from_millis(1490), debounce));
        assert!(!ctrl.is_dormant);

        // After 1.5s: enters dormant
        assert!(ctrl.check_enter_dormant(true, t1 + Duration::from_millis(1501), debounce));
        assert!(ctrl.is_dormant);

        // Re-trim interval 20s
        assert!(!ctrl.should_periodic_trim(
            t1 + Duration::from_millis(1501) + Duration::from_secs(19),
            Duration::from_secs(20)
        ));
        assert!(ctrl.should_periodic_trim(
            t1 + Duration::from_millis(1501) + Duration::from_secs(21),
            Duration::from_secs(20)
        ));

        // Unminimize: cancelled immediately (<50ms synchronous)
        let unminimized = ctrl.on_window_state_change(false, t1 + Duration::from_secs(25));
        assert!(unminimized);
        assert!(!ctrl.is_dormant);
    }

    #[test]
    fn dormant_controller_qos_throttling_rules() {
        // Not dormant: nobody throttled
        assert!(!DormantController::should_throttle_process(false, true, false));
        assert!(!DormantController::should_throttle_process(false, false, false));

        // Dormant + Audio playing: child throttled, root NOT throttled!
        assert!(!DormantController::should_throttle_process(true, true, true));
        assert!(DormantController::should_throttle_process(true, false, true));

        // Dormant + Audio paused: child throttled, root throttled
        assert!(DormantController::should_throttle_process(true, true, false));
        assert!(DormantController::should_throttle_process(true, false, false));
    }
}
