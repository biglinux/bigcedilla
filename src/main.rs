//! `bigcedilla`: KWin/Plasma 6 input-method-v1 MITM proxy. Owns the IM slot,
//! forwards everything to a real virtual keyboard child (plasma-keyboard by
//! default, maliit-keyboard via `BIGCEDILLA_CHILD_IM=maliit-keyboard`), and
//! intercepts `dead_acute + c` at the protocol layer so it commits U+00E7
//! (`ç`) instead of the broken U+0107 (`ć`) seen on Chromium-based browsers.

mod compose;
mod proxy;
mod spawn;

use anyhow::{Context, Result};
use wl_proxy::baseline::Baseline;
use wl_proxy::simple::SimpleProxy;

fn main() -> Result<()> {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    log::info!(
        "bigcedilla starting pid={} ppid={}",
        std::process::id(),
        std::os::unix::process::parent_id()
    );
    for var in ["WAYLAND_DISPLAY", "WAYLAND_SOCKET", "XDG_RUNTIME_DIR"] {
        log::info!("env {var}={:?}", std::env::var(var).ok());
    }

    let child_bin = spawn::pick_child_binary().context("no child IM available")?;
    log::info!("child IM resolved to {}", child_bin.display());

    let server = SimpleProxy::new(Baseline::V3).context("failed to create wl-proxy server")?;
    let display_name = server.display().to_owned();
    log::info!("proxy listening on {display_name}");

    let mut child = spawn::spawn_child(&child_bin, &display_name)
        .context("failed to spawn child virtual keyboard")?;
    log::info!("child pid={} spawned", child.id());

    // Reap zombie if the child dies while the proxy keeps running.
    std::thread::spawn(move || {
        if let Ok(status) = child.wait() {
            log::warn!("child IM exited with {status}");
        }
    });

    let err = server.run(proxy::DisplayHandler::new);
    Err(anyhow::Error::new(err).context("wl-proxy server terminated"))
}
