#!/usr/bin/env bash
# Probe whether plasma-keyboard (Qt6 VirtualKeyboard) sees hardware key events
# delivered via zwp_input_method_v1 on KWin Plasma 6.
#
# Usage:
#   sudo ./probe-plasma-keyboard.sh install    # install debug wrapper + register IM
#   sudo ./probe-plasma-keyboard.sh restore    # revert to previous IM (Maliit by default)
#   ./probe-plasma-keyboard.sh tail            # follow live log (no sudo)
#
# After 'install': run `qdbus6 org.kde.KWin /KWin reconfigure`, focus a text input
# in Brave, type the dead_acute key (´) followed by `c`. Observe /tmp/plasma-kb-probe.log.
#
# Signals to look for in the log:
#   * "zwp_input_method_v1@... activate"          -> KWin spawned + activated us
#   * "zwp_input_method_context_v1@... grab_keyboard" -> we grabbed hardware kb
#   * "wl_keyboard@... key, ..."                  -> hardware keys reach plasma-keyboard
#   * "wl_keyboard@... keymap"                    -> xkb keymap delivered
#   * QtVirtualKeyboard "keyEvent" lines (qt.virtualkeyboard.*)
#
# If grab_keyboard appears AND key events flow AND keyEvent log lines fire,
# a Qt VKB plugin can intercept dead_acute+c. Otherwise the daemon route stays.

set -euo pipefail

LOG=/tmp/plasma-kb-probe.log
WRAPPER=/usr/local/bin/plasma-keyboard-probe
DESKTOP=/usr/share/applications/plasma-keyboard-probe.desktop
KWINRC_KEY="InputMethod"
KWINRC_GROUP="Wayland"
SAVED_IM_FILE=/var/lib/plasma-kb-probe.previous

cmd=${1:-}

case "$cmd" in
install)
    [[ $EUID -eq 0 ]] || { echo "needs sudo"; exit 1; }

    user=${SUDO_USER:-}
    [[ -n $user ]] || { echo "run via sudo from a desktop user shell"; exit 1; }

    mkdir -p /var/lib

    # save current IM setting (per-user) so we can restore
    su - "$user" -c "kreadconfig6 --file kwinrc --group $KWINRC_GROUP --key $KWINRC_KEY" \
        > "$SAVED_IM_FILE" || true
    echo "previous IM: $(cat "$SAVED_IM_FILE")"

    cat > "$WRAPPER" <<'EOF'
#!/usr/bin/env bash
exec env \
    QT_LOGGING_RULES='qt.virtualkeyboard*=true;qt.qpa.input.*=true' \
    WAYLAND_DEBUG=1 \
    plasma-keyboard "$@" >> /tmp/plasma-kb-probe.log 2>&1
EOF
    chmod 0755 "$WRAPPER"

    cat > "$DESKTOP" <<EOF
[Desktop Entry]
Name=PlasmaKeyboardProbe
Exec=$WRAPPER
Type=Application
X-KDE-Wayland-VirtualKeyboard=true
Icon=input-keyboard-virtual
NoDisplay=true
EOF
    chmod 0644 "$DESKTOP"

    : > "$LOG"
    chmod 0666 "$LOG"

    su - "$user" -c "kwriteconfig6 --file kwinrc --group $KWINRC_GROUP --key $KWINRC_KEY $DESKTOP"

    cat <<EOF

Probe wrapper installed.
Next:
  1. Run as desktop user (NOT sudo):
        qdbus6 org.kde.KWin /KWin reconfigure
  2. Open Brave / Chrome, focus a text input.
  3. Type:  acute-key (\`'\` on US-intl, \`´\` on pt_BR) then 'c'.
  4. Tail the log:
        $0 tail
  5. When done, restore Maliit:
        sudo $0 restore
        qdbus6 org.kde.KWin /KWin reconfigure
EOF
    ;;

tail)
    [[ -f $LOG ]] || { echo "no log yet — run install + reconfigure first"; exit 1; }
    exec tail -F "$LOG"
    ;;

restore)
    [[ $EUID -eq 0 ]] || { echo "needs sudo"; exit 1; }
    user=${SUDO_USER:-}
    [[ -n $user ]] || { echo "run via sudo from a desktop user shell"; exit 1; }

    prev=$(cat "$SAVED_IM_FILE" 2>/dev/null || echo "/usr/share/applications/com.github.maliit.keyboard.desktop")
    [[ -n $prev ]] || prev="/usr/share/applications/com.github.maliit.keyboard.desktop"

    su - "$user" -c "kwriteconfig6 --file kwinrc --group $KWINRC_GROUP --key $KWINRC_KEY $prev"
    rm -f "$WRAPPER" "$DESKTOP" "$SAVED_IM_FILE"

    echo "restored IM to: $prev"
    echo "run: qdbus6 org.kde.KWin /KWin reconfigure"
    ;;

*)
    grep '^# ' "$0" | sed 's/^# \?//' | head -20
    exit 1
    ;;
esac
