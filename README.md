# Zenith

Clip the last minute of any game on Linux. A small background recorder keeps
a rolling buffer while a game runs; press your shortcut and it is saved as a
clip. The app is where you browse clips, trim and cut them, and export them
at a size that fits Discord.

- **Light.** The recorder is a few megabytes and does nothing until a game
  starts. Capture and encoding happen on the GPU, through
  [gpu-screen-recorder](https://git.dec05eba.com/gpu-screen-recorder/about/).
- **Three sound tracks.** Game, voice chat and microphone are recorded
  apart, so each can be turned down or muted afterwards.
- **An editor that does not touch your files.** Trim, cut out as many parts
  as you like, undo anything. The original clip stays as it was.
- **Export to a size.** 20 MB for Discord, 50 or 500 MB for Nitro, or any
  size: resolution and frame rate are picked to fit, and the result is
  copied, ready to paste.
- **No account, and your clips stay on your computer.** The one thing Zenith
  contacts is GitHub, to see whether there is a newer version; that can be
  switched off in the settings.

## Install

Needs `gpu-screen-recorder`, `ffmpeg`, `mpv`, Qt 6 (`qt6-declarative`) and
PipeWire, and Rust to build.

On Arch and its relatives:

```sh
cd dist/arch && makepkg -si
systemctl --user enable --now zenithd
```

Anywhere else, for your own user:

```sh
./dist/install.sh
```

Then bind a key to `zenithd save` in your desktop's keyboard settings,
and open Zenith from the launcher. `zenithd doctor` says whether
everything is in place.

## Using it

| | |
|---|---|
| `zenithd save` | save the last seconds as a clip (your shortcut) |
| `zenithd status` | what the recorder is doing |
| `zenithd capture on\|off\|auto` | capture always, never, or while a game runs |
| `zenithd doctor` | check the setup |

Clips go to `~/Videos/Clips/<Game>/`. Settings are in the app, and in
`~/.config/zenith-clips/config.toml`.

Games are found through Steam (and, on Windows, the other launchers); for anything else, set Capture to
"Always" in the settings.

## Working on it

```
crates/core     the logic, with no UI toolkit: settings, recorder state, library, edits, export
crates/daemon   zenithd, the background recorder
app             zenith-clips, the window (Rust + QML, video through libmpv)
dist            install script, systemd unit, launcher entry, Arch package
tests/fake-gsr  a stand-in recorder for the daemon's tests
```

```sh
cargo test --workspace                              # everything, including real ffmpeg exports
cargo clippy --workspace --all-targets -- -D warnings
cargo run -p zenith -- --shot /tmp/window.png   # a picture of the window, then quit
cargo run -p zenith -- --scrolltest              # run the library top to bottom; says how fast it opened and scrolled
cargo run -p zenith -- --librarytest a.mp4      # pick, tag, rename, move, trash and undo, on a clip the test brings in itself
```
