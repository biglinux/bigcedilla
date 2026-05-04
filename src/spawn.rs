//! Child IM spawn: launch `plasma-keyboard` or `maliit-keyboard` pointing at
//! the proxy display.
//!
//! Selection priority:
//!   1. `BIGCEDILLA_CHILD_IM` env var (absolute path or basename in `/usr/bin`)
//!   2. `/usr/bin/plasma-keyboard` if it exists (default — KDE Plasma 6 stock)
//!   3. `/usr/bin/maliit-keyboard` (fallback; opt in explicitly via
//!      `BIGCEDILLA_CHILD_IM=maliit-keyboard` if both are installed)
//!
//! The child inherits everything except `WAYLAND_SOCKET` (cleared so it does
//! not reconnect to `KWin`'s private fd) and `WAYLAND_DISPLAY` (set to the
//! proxy socket).

use std::path::{Path, PathBuf};
use std::process::{Child, Command};

use anyhow::{Context, Result, anyhow};

const ENV_OVERRIDE: &str = "BIGCEDILLA_CHILD_IM";
const CANDIDATES: &[&str] = &["/usr/bin/plasma-keyboard", "/usr/bin/maliit-keyboard"];

pub fn pick_child_binary() -> Result<PathBuf> {
    if let Ok(custom) = std::env::var(ENV_OVERRIDE) {
        let path = if custom.contains('/') {
            PathBuf::from(custom)
        } else {
            PathBuf::from("/usr/bin").join(custom)
        };
        if path.exists() {
            return Ok(path);
        }
        return Err(anyhow!(
            "{ENV_OVERRIDE} points to {} which does not exist",
            path.display()
        ));
    }
    for candidate in CANDIDATES {
        let path = Path::new(candidate);
        if path.exists() {
            return Ok(path.to_path_buf());
        }
    }
    Err(anyhow!(
        "no virtual keyboard found; install plasma-keyboard or maliit-keyboard"
    ))
}

pub fn spawn_child(binary: &Path, proxy_display: &str) -> Result<Child> {
    log::info!("spawning child IM {} on {proxy_display}", binary.display());
    Command::new(binary)
        .env("WAYLAND_DISPLAY", proxy_display)
        .env_remove("WAYLAND_SOCKET")
        .spawn()
        .with_context(|| format!("failed to spawn {}", binary.display()))
}
