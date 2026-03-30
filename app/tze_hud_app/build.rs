// build.rs — tze_hud_app
//
// On Windows: embed the DPI-awareness application manifest so the overlay covers
// the full physical display at any DPI scaling level, including when launched
// from a scheduled task without an interactive console session.
//
// The manifest (`tze_hud.manifest`) declares `PerMonitorV2` DPI awareness.
// This is processed by the Windows OS loader before any code runs, which
// guarantees that:
//   - `MonitorHandle::size()` returns physical pixels (not logical/virtualised)
//   - `window.inner_size()` returns physical pixels
//   - `GetDpiForMonitor(MDT_EFFECTIVE_DPI)` returns the real monitor DPI
//
// Without this manifest, processes launched by the Task Scheduler may start
// in a DPI-unaware context, causing virtualised pixel dimensions (e.g. a
// 2560×1440 display at 125% DPI reports 2048×1152 to the process).
//
// See: hud-22by — Overlay window doesn't cover full display at >100% DPI scaling

fn main() {
    // On non-Windows targets this block compiles to nothing.
    #[cfg(target_os = "windows")]
    {
        embed_resource::compile("tze_hud.manifest", embed_resource::NONE);
    }
}
