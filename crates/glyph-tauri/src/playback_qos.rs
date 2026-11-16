//! Keep background playback eligible for timely CPU scheduling. Windows can
//! otherwise lower QoS when every window is minimized or fully covered.

use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Threading::{
    GetCurrentProcess, GetCurrentProcessId, OpenProcess, ProcessPowerThrottling, SetProcessInformation,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE, PROCESS_SET_INFORMATION,
};

pub fn configure() {
    // Apply separately: older Windows versions may reject the timer flag, but
    // should still receive the supported execution-speed opt-out. This does
    // not raise process priority or request a higher timer resolution itself.
    for (mechanism, name) in [
        (PROCESS_POWER_THROTTLING_EXECUTION_SPEED, "execution speed"),
        (PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, "timer resolution"),
    ] {
        let policy = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: mechanism,
            StateMask: 0,
        };
        // SAFETY: the pseudo-handle refers to this process, and policy remains
        // alive for the synchronous call with its exact structure size.
        let result = unsafe {
            SetProcessInformation(
                GetCurrentProcess(),
                ProcessPowerThrottling,
                (&policy as *const PROCESS_POWER_THROTTLING_STATE).cast(),
                std::mem::size_of_val(&policy) as u32,
            )
        };
        match result {
            Ok(()) => tracing::info!(mechanism = name, "background playback throttling disabled"),
            Err(error) => tracing::warn!(mechanism = name, %error, "cannot disable background playback throttling"),
        }
    }
}

/// Computes the `(ControlMask, StateMask)` flags for Windows Power Throttling.
#[inline]
pub fn compute_throttling_policy(enable: bool, include_timer: bool) -> (u32, u32) {
    let control = if include_timer {
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION
    } else {
        PROCESS_POWER_THROTTLING_EXECUTION_SPEED
    };
    let state = if enable { control } else { 0 };
    (control, state)
}

/// Controls Windows EcoQoS (Efficiency Mode / Power Throttling) for a process.
///
/// When `enable` is true:
/// Enables `PROCESS_POWER_THROTTLING_EXECUTION_SPEED` and
/// `PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION`.
///
/// When `enable` is false:
/// Disables power throttling, ensuring high-priority audio processing.
pub fn set_process_efficiency_mode(pid: u32, enable: bool) -> bool {
    let current_pid = unsafe { GetCurrentProcessId() };
    let (handle, need_close) = if pid == current_pid {
        (unsafe { GetCurrentProcess() }, false)
    } else {
        // SAFETY: OpenProcess with PROCESS_SET_INFORMATION allows configuring power throttling.
        match unsafe { OpenProcess(PROCESS_SET_INFORMATION, false, pid) } {
            Ok(h) => (h, true),
            Err(e) => {
                tracing::trace!(pid, %e, "cannot open process for QoS configuration");
                return false;
            }
        }
    };

    let (control, state) = compute_throttling_policy(enable, true);
    let policy = PROCESS_POWER_THROTTLING_STATE {
        Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
        ControlMask: control,
        StateMask: state,
    };

    // SAFETY: `handle` is valid and `policy` matches `PROCESS_POWER_THROTTLING_STATE`.
    let mut ok = unsafe {
        SetProcessInformation(
            handle,
            ProcessPowerThrottling,
            (&policy as *const PROCESS_POWER_THROTTLING_STATE).cast(),
            std::mem::size_of_val(&policy) as u32,
        )
    }
    .is_ok();

    if !ok {
        // Fallback for Windows versions where timer resolution throttling is rejected.
        let (fallback_control, fallback_state) = compute_throttling_policy(enable, false);
        let fallback_policy = PROCESS_POWER_THROTTLING_STATE {
            Version: PROCESS_POWER_THROTTLING_CURRENT_VERSION,
            ControlMask: fallback_control,
            StateMask: fallback_state,
        };
        ok = unsafe {
            SetProcessInformation(
                handle,
                ProcessPowerThrottling,
                (&fallback_policy as *const PROCESS_POWER_THROTTLING_STATE).cast(),
                std::mem::size_of_val(&fallback_policy) as u32,
            )
        }
        .is_ok();
    }

    if need_close {
        // SAFETY: `handle` was returned by `OpenProcess`.
        unsafe {
            let _ = CloseHandle(handle);
        }
    }

    tracing::debug!(pid, enable, ok, "set_process_efficiency_mode");
    ok
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_compute_throttling_policy() {
        let (ctrl, state) = compute_throttling_policy(true, true);
        assert_eq!(
            ctrl,
            PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION
        );
        assert_eq!(state, ctrl);

        let (ctrl, state) = compute_throttling_policy(false, true);
        assert_eq!(
            ctrl,
            PROCESS_POWER_THROTTLING_EXECUTION_SPEED | PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION
        );
        assert_eq!(state, 0);

        let (ctrl, state) = compute_throttling_policy(true, false);
        assert_eq!(ctrl, PROCESS_POWER_THROTTLING_EXECUTION_SPEED);
        assert_eq!(state, PROCESS_POWER_THROTTLING_EXECUTION_SPEED);

        let (ctrl, state) = compute_throttling_policy(false, false);
        assert_eq!(ctrl, PROCESS_POWER_THROTTLING_EXECUTION_SPEED);
        assert_eq!(state, 0);
    }

    #[test]
    fn test_set_process_efficiency_mode_self() {
        let pid = unsafe { GetCurrentProcessId() };
        let ok_on = set_process_efficiency_mode(pid, true);
        assert!(ok_on);
        let ok_off = set_process_efficiency_mode(pid, false);
        assert!(ok_off);
    }
}
