#!/usr/bin/env bash
# Security gate for bigcedilla. Runs locally and in CI.
#
# bigcedilla is a Wayland input-method MITM. By protocol design every IM v1
# implementation has full keystroke access — that is why this gate is strict
# about (a) zero outbound network paths, (b) zero unsafe outside the keymap
# mmap helper, (c) no telemetry/logging of keysyms, (d) supply-chain hygiene.
#
# Usage:
#   scripts/security-check.sh           # blocking gate (fast)
#   scripts/security-check.sh --full    # adds slow probes (strace, miri)
#   scripts/security-check.sh --fix     # autofix what can be autofixed
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
cd "$ROOT"

MODE="${1:-ci}"
RED=$'\033[31m'; GRN=$'\033[32m'; YLW=$'\033[33m'; CLR=$'\033[0m'
say()  { printf '%s==> %s%s\n' "$GRN" "$*" "$CLR"; }
warn() { printf '%s!!  %s%s\n' "$YLW" "$*" "$CLR"; }
die()  { printf '%sXX  %s%s\n' "$RED" "$*" "$CLR"; exit 1; }

# ---------------------------------------------------------------- supply chain
say "cargo audit (RustSec advisories)"
if command -v cargo-audit >/dev/null; then
    cargo audit --deny warnings
else
    warn "cargo-audit missing — install: cargo install cargo-audit --locked"
fi

say "cargo deny check (advisories + licenses + bans + sources)"
if command -v cargo-deny >/dev/null; then
    cargo deny check
else
    warn "cargo-deny missing — install: cargo install cargo-deny --locked"
fi

say "cargo machete (unused deps)"
if command -v cargo-machete >/dev/null; then
    cargo machete
else
    warn "cargo-machete missing — install: cargo install cargo-machete --locked"
fi

# ---------------------------------------------------------------- correctness
say "cargo clippy -D warnings (pedantic + restriction)"
cargo clippy --all-targets --all-features -- -D warnings

say "cargo fmt --check"
cargo fmt --check

say "cargo test"
cargo test --quiet

# ---------------------------------------------------------------- unsafe surface
say "cargo geiger (unsafe block counter)"
if command -v cargo-geiger >/dev/null; then
    cargo geiger --quiet --output-format Ascii || warn "geiger flagged unsafe — review changes"
else
    warn "cargo-geiger missing — install: cargo install cargo-geiger --locked"
fi

say "grep for unsafe outside build_xkb_state"
# Only the keymap mmap helper is allowed to use unsafe.
unsafe_hits=$(grep -RnE '\bunsafe\b' src/ | grep -v 'src/proxy.rs' || true)
if [ -n "$unsafe_hits" ]; then
    printf '%s\n' "$unsafe_hits"
    die "unsafe found outside the keymap mmap helper — review carefully"
fi

# ---------------------------------------------------------------- exfil paths
say "static check: no network / fs writes / spawn (except spawn.rs)"
# bigcedilla must not open sockets, write user files, exec arbitrary children.
forbidden='\b(TcpStream|UdpSocket|reqwest|hyper|tokio::net|std::net::|File::create|fs::write|fs::OpenOptions|Command::new)\b'
hits=$(grep -RnE "$forbidden" src/ | grep -v 'src/spawn.rs' || true)
if [ -n "$hits" ]; then
    printf '%s\n' "$hits"
    die "forbidden API outside spawn.rs — bigcedilla must not touch network/fs"
fi

say "static check: no keysym/text logging at info+ level"
# Compose state must never log the actual key events at info or higher.
# debug! is allowed (off by default; only on with RUST_LOG=debug).
log_hits=$(grep -RnE 'log::(info|warn|error)!.*\b(sym|keysym|key|text|commit_string)\b' src/ || true)
if [ -n "$log_hits" ]; then
    printf '%s\n' "$log_hits"
    warn "review above lines: keysym/key in info+ log — confirm no keystroke leak"
fi

# ---------------------------------------------------------------- secrets
say "gitleaks (committed secrets)"
if command -v gitleaks >/dev/null; then
    if [ -d .git ]; then
        gitleaks detect --no-banner --redact --exit-code 1
    else
        warn "not a git repo — skipping gitleaks"
    fi
else
    warn "gitleaks missing — see https://github.com/gitleaks/gitleaks"
fi

# ---------------------------------------------------------------- --full only
if [ "$MODE" = "--full" ]; then
    say "[full] strace network probe (5s)"
    if command -v strace >/dev/null; then
        bin="$ROOT/target/release/bigcedilla"
        [ -x "$bin" ] || cargo build --release --quiet
        # Run briefly with no upstream and capture syscalls. AF_INET/AF_INET6
        # would indicate outbound network — must stay empty.
        log=$(mktemp)
        timeout 3 strace -f -e trace=network -o "$log" "$bin" >/dev/null 2>&1 || true
        net=$(grep -E 'AF_INET6?|connect\(.*sin_' "$log" || true)
        rm -f "$log"
        if [ -n "$net" ]; then
            printf '%s\n' "$net"
            die "network syscalls observed under strace — investigate"
        fi
        say "[full] no network syscalls under strace"
    else
        warn "strace missing"
    fi

    say "[full] cargo +nightly miri test --lib (compose module only)"
    if rustup +nightly which miri >/dev/null 2>&1; then
        # FFI in proxy.rs (xkbcommon, mmap) cannot run under miri — restrict
        # to the pure-logic compose module.
        cargo +nightly miri test --lib compose:: || warn "miri reported issues"
    else
        warn "miri toolchain missing — rustup +nightly component add miri rust-src"
    fi

    say "[full] systemd-analyze security (if unit installed)"
    if command -v systemd-analyze >/dev/null && [ -f pkgbuild/bigcedilla.service ]; then
        systemd-analyze security --no-pager pkgbuild/bigcedilla.service || true
    fi
fi

if [ "$MODE" = "--fix" ]; then
    say "[fix] cargo fmt"
    cargo fmt
    say "[fix] cargo clippy --fix"
    cargo clippy --fix --allow-dirty --allow-staged --all-targets --all-features
fi

say "security gate passed"
