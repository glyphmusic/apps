//! Keep background playback eligible for timely CPU scheduling. Windows can
//! otherwise lower QoS when every window is minimized or fully covered.

use windows::Win32::System::Threading::{
    GetCurrentProcess, ProcessPowerThrottling, SetProcessInformation,
    PROCESS_POWER_THROTTLING_CURRENT_VERSION, PROCESS_POWER_THROTTLING_EXECUTION_SPEED,
    PROCESS_POWER_THROTTLING_IGNORE_TIMER_RESOLUTION, PROCESS_POWER_THROTTLING_STATE,
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
