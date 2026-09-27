//! OS-specific tuning for accurate scheduling.

/// Asks Windows for 1 ms timer resolution for this process. Without it, timers wake on the default
/// ~15.6 ms tick, so an open-model scheduler sends in bursts and that lateness shows up as latency.
/// Idempotent; no-op elsewhere.
pub fn high_res_timer() {
    #[cfg(windows)]
    {
        static ONCE: std::sync::Once = std::sync::Once::new();
        ONCE.call_once(|| {
            // SAFETY: plain FFI call with a valid argument; paired end call isn't needed because the
            // setting is released when the process exits.
            let rc = unsafe { windows_sys::Win32::Media::timeBeginPeriod(1) };
            if rc != 0 {
                tracing::warn!("timeBeginPeriod(1) failed ({rc}); open-model scheduling will be coarse");
            }
        });
    }
}
