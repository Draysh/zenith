#!/bin/sh
# Build Zenith and install it for this user: the recorder and the
# app in ~/.local/bin, the recorder started with the desktop session, the
# app in the launcher.
set -eu
cd "$(dirname "$0")/.."
cargo build --release --locked -p zenith-daemon -p zenith
install -Dm755 target/release/zenithd "$HOME/.local/bin/zenithd"
install -Dm755 target/release/zenith-clips "$HOME/.local/bin/zenith-clips"
# The launcher's PATH often lacks ~/.local/bin, so point Exec at the file.
mkdir -p "$HOME/.local/share/applications"
sed "s|^Exec=zenith-clips\$|Exec=$HOME/.local/bin/zenith-clips|" dist/zenith-clips.desktop \
    > "$HOME/.local/share/applications/zenith-clips.desktop"
install -Dm644 dist/icons/zenith-clips.svg "$HOME/.local/share/icons/hicolor/scalable/apps/zenith-clips.svg"
# Without systemd for the user, the recorder is left to be started with the
# session by whatever starts things there.
if systemctl --user show-environment >/dev/null 2>&1; then
    install -Dm644 dist/zenithd.service "$HOME/.config/systemd/user/zenithd.service"
    systemctl --user daemon-reload
    systemctl --user enable zenithd.service
    systemctl --user restart zenithd.service
else
    echo "No systemd user session here: start $HOME/.local/bin/zenithd with your desktop session."
fi
echo "Installed. Bind a key to:  $HOME/.local/bin/zenithd save"
