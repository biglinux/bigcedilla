# BigCedilla

A KWin/Plasma 6 input-method-v1 proxy that restores the Brazilian
**ç** when a dead-acute key (`'` or `´`) is followed by **c** — without
disabling the on-screen keyboard (plasma-keyboard or Maliit).

---

## Why this exists

Chromium and its derivatives (Google Chrome, Brave, Edge, Opera,
Vivaldi — anything that renders with `WebContents`) **no longer
accept the `'` + `c` sequence as `ç`** on Brazilian / US-International
layouts under Wayland.

Expected behavior:

```
'  +  c   →   ç
'  +  C   →   Ç
```

Current behavior in Chromium:

```
'  +  c   →   ć   (U+0107, c with acute — Polish letter)
```

Cause: Chromium now combines the dead acute directly onto the `c`,
ignoring the Brazilian `XCompose` rule that maps that sequence to
`ç` (U+00E7). It affects both **US-International with dead keys** and
**pt-BR** variants that emit `dead_acute`.

Other browsers (Firefox, WebKit/GNOME Web) still respect `XCompose`
and are unaffected. Native GTK/Qt apps, IDEs and text editors keep
working as expected. The regression is limited to Chromium and
everything that inherits from it.

`bigcedilla` fixes the problem **outside the browser**, at the
Wayland protocol layer — so it works for every app on the system,
not only Chromium.

---

## How it works

```
┌─────────┐   IM v1    ┌─────────────┐   IM v1    ┌──────────────────┐
│  KWin   │ ─────────► │ bigcedilla  │ ─────────► │ plasma-keyboard  │
│ Plasma6 │            │   (proxy)   │            │ or maliit-kbd    │
└─────────┘ ◄───────── └─────────────┘ ◄───────── └──────────────────┘
                              │
                              │ intercept dead_acute + c
                              ▼
                     commit_string("ç") → KWin
```

1. KWin treats `bigcedilla` as the system input method (the
   `[Wayland] InputMethod` field in `kwinrc`).
2. `bigcedilla` opens its own private Wayland socket and starts the
   real on-screen keyboard (plasma-keyboard by default; switch to
   Maliit via `BIGCEDILLA_CHILD_IM=maliit-keyboard`).
3. Everything the on-screen keyboard emits is forwarded as-is.
   Touch typing keeps working.
4. In parallel, `bigcedilla` calls `grab_keyboard` on the IM context
   and runs hardware keysyms through a tiny compose state machine.
   Only the `dead_acute + c` sequence is intercepted and rewritten as
   `commit_string("ç")` (or `"Ç"` for capital `c`).
5. Every other key is replayed via `context.key()` so the focused
   app processes it normally — including its own `XCompose`,
   shortcuts, etc.

---

## Install

### Arch / BigLinux / Manjaro

```bash
cd pkgbuild
makepkg -si
```

Then, as the desktop user (not root):

```bash
bigcedilla-configure-kwin
qdbus6 org.kde.KWin /KWin reconfigure
```

To revert at any time:

```bash
bigcedilla-restore-maliit
qdbus6 org.kde.KWin /KWin reconfigure
```

### Manual build

```bash
cargo build --release
sudo install -Dm755 target/release/bigcedilla /usr/bin/bigcedilla
sudo install -Dm644 pkgbuild/bigcedilla.desktop \
    /usr/share/applications/bigcedilla.desktop
sudo install -Dm755 pkgbuild/bigcedilla-configure-kwin /usr/bin/
sudo install -Dm755 pkgbuild/bigcedilla-restore-maliit /usr/bin/
```

---

## Configuration

| Environment variable     | Effect                                                 |
|--------------------------|--------------------------------------------------------|
| `BIGCEDILLA_CHILD_IM`    | Path or basename of the child on-screen keyboard. Default: `plasma-keyboard`, fallback `maliit-keyboard`. Set to `maliit-keyboard` to force Maliit when both are installed. |
| `BIGCEDILLA_FORCE`       | `1` always activate, `0` always skip. Bypasses the layout/locale check. |
| `RUST_LOG=debug`         | Verbose log (includes keysyms — local debugging only). |

### Layout / locale gating

`bigcedilla-check-layout` decides whether the proxy is useful for the
current account. It returns success when any of the following holds:

- `LANG` / `LC_*` matches `pt_*`, `ca_*`, `oc_*`, `fur_*`
- KDE `kxkbrc` (or `localectl` / `setxkbmap` fallback) reports a layout
  in `{br, pt, ca, oc}`
- a `us` layout uses an `intl` or `altgr-intl` variant

It is wired in two places:

- The user systemd unit (`pkgbuild/bigcedilla.service`) calls it as
  `ExecCondition=` — if the layout does not benefit, the unit silently
  exits without spawning the proxy.
- `bigcedilla-configure-kwin` refuses to register `bigcedilla` as the
  KWin input method when the check fails, unless invoked with
  `--force` or `BIGCEDILLA_FORCE=1`.

Layouts with `ç` as a direct key (French AZERTY, Turkish, Albanian,
Azeri, Tatar) are intentionally skipped — they were never affected by
the Chromium regression.

---

## Requirements

- KWin / Plasma 6 in a Wayland session
- `plasma-keyboard` **or** `maliit-keyboard` installed
- `libxkbcommon`, `wayland`

---

## License

GPL-3.0-or-later.
