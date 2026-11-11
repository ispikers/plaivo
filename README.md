# Plaivo

A native Plex and Jellyfin client for Windows, macOS, Android and Android TV.

The desktop app is Rust. Zed's GPUI draws the interface, and libmpv renders the video in its own window underneath it, so the file plays the way mpv would play it: anything FFmpeg can decode, HDR passed through to the display, hardware decoding, mpv's frame pacing. The Android app is Kotlin with Compose and ExoPlayer, on top of the same Rust core that the desktop uses for the Plex and Jellyfin clients, settings, downloads, tracker sync and Watch Together. There is no browser engine in either.

There are no releases yet. See [Status](#status) for what works and what hasn't been checked.

## What it does

- Plex (PIN sign-in, every server on the account) and Jellyfin (password or Quick Connect), listed side by side in one sidebar.
- Direct play by default. Quality presets ask the server to transcode when the connection can't keep up.
- Downloads for offline viewing. Progress made offline is sent to the server, and to Trakt or MyAnimeList, once you're back online.
- Skip intro and credits from Plex markers, an Up next countdown, the show's episodes as a queue.
- A miniplayer, so playback continues while you browse.
- Watch Together over the openwatchparty protocol, so rooms are shared with Plezy users.
- Trakt and MyAnimeList scrobbling.
- mpv GLSL upscalers on the desktop: Anime4K, ArtCNN and NVScaler.
- Preferred audio and subtitle languages read from, and written back to, your Plex or Jellyfin account.

## Status

Not there yet:

- An installer or portable zip
- An app icon
- Mark as watched
- Chapters
- Trailers
- Plex Home user switching
- Media keys
- Intel Macs: macOS builds are Apple Silicon only, because the pinned libmpv is built for arm64

## Building the desktop app

You need Rust 1.85 or newer, and the libmpv build pinned in `deps/libmpv.env` (currently mpv 0.42.0, FFmpeg 8.1.3, libplacebo 7.387.0):

```
cargo run -p xtask -- libmpv
```

This downloads Plaivo's own libmpv archive for your OS from this repository's releases, checks its SHA-256 against the pin, and unpacks it into `deps/windows/` or `deps/macos/`. The repository is private, so it needs a GitHub token in `GITHUB_TOKEN` or `GH_TOKEN`, or one that `git credential fill` can provide.

Then build as usual. `build.rs` copies libmpv and `shaders/` next to the executable.

```
cargo build --release
```

On macOS, run Plaivo from an app bundle. A bare binary started from Finder runs inside Terminal instead of as its own app.

```
cargo run -p xtask -- bundle --release --open
```

That writes `target/release/Plaivo.app` (ad-hoc signed, macOS 15 or later).

To build libmpv itself, see `deps/build-libmpv.sh` and the `libmpv` workflow in `.github/workflows/`. Any change to the pins needs a new `TAG`.

## Building the Android app

One APK covers phones, tablets and Android TV. The layout is chosen from the device type at launch. Android 7.0 (API 24) or newer, arm64 and armv7.

You need the Android SDK, the NDK version named in `android/app/build.gradle.kts`, JDK 17, and:

```
rustup target add aarch64-linux-android armv7-linux-androideabi
cargo install cargo-ndk
```

From `android/`:

```
gradlew.bat assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

Use `./gradlew` outside Windows. `preBuild` compiles the Rust library with `cargo ndk` and generates the Kotlin bindings with UniFFI, so the first build takes a while. `assembleRelease` gives an unsigned APK; sign it with your own key using `apksigner`, or use Android Studio's Generate Signed App Bundle or APK. Formatting is `gradlew spotlessApply`, lint is `gradlew lintDebug`.

## Build-time variables

Both are read at compile time and never committed. The Android Rust build inherits Gradle's environment.

| Variable | Used for |
| --- | --- |
| `PLAIVO_TRAKT_ID` | The Trakt app's client ID. Without it, Settings → Services shows Trakt as "Not configured". |
| `PLAIVO_OAUTH_RELAY` | Overrides the MyAnimeList sign-in relay. Defaults to `https://relay.plaivo.app` (`workers/oauth-relay`). |

## Where things are kept

| | Windows | macOS |
| --- | --- | --- |
| Account and settings | `%APPDATA%\Plaivo` | `~/Library/Application Support/Plaivo` |
| Cache | `%LOCALAPPDATA%\Plaivo\cache` | `~/Library/Caches/Plaivo/cache` |
| Logs | `%LOCALAPPDATA%\Plaivo\logs\plaivo.log` | `~/Library/Caches/Plaivo/logs/plaivo.log` |

Set `PLAIVO_LOG=debug` for verbose logs, or turn on Settings → Advanced → Debug logging. Plex tokens are always redacted. An `mpv.conf` in the settings folder is read by mpv on start.

## Player keys

| Key | Action |
| --- | --- |
| Space, K | Play or pause |
| ← / → | Seek by the small step (10 s by default) |
| Shift+← / → | Seek by the large step (30 s by default) |
| ↑ / ↓ | Volume |
| M | Mute |
| F | Fullscreen |
| S | Skip the intro or credits |
| N / P | Next or previous episode |
| Esc | Close a panel, leave fullscreen, then shrink to the miniplayer |

## Tests

```
cargo test --workspace
```

Four live tests are marked `#[ignore]` because they go to the network (Watch Together on the public relay, Plex language prefs and range reads, the anime ID mappings). Run them with `cargo test --workspace -- --ignored`. The relay Worker has its own tests: `bun test` in `workers/oauth-relay`.

## Layout

| Path | |
| --- | --- |
| `src/` | Desktop app. The UI is in `src/ui/`, mpv in `mpv.rs` and `player.rs`, the video window in `video_host.rs`. |
| `crates/core` | The shared core: Plex and Jellyfin clients, storage, settings, downloads, trackers, Watch Together. No UI. |
| `crates/ffi` | The UniFFI surface the Android app calls. |
| `android/` | The Android and Android TV app. |
| `xtask/` | `libmpv` and `bundle` commands. |
| `deps/` | libmpv pins, build script and the macOS embedding patch. |
| `shaders/` | mpv GLSL shaders, MIT, from their upstream repositories. |
| `licenses/` | Generates the third-party license list shown in the app. |
| `workers/oauth-relay` | Cloudflare Worker for MyAnimeList sign-in. |
| `workers/site` | plaivo.app, a static site on Cloudflare Workers. |

## Licensing

Plaivo itself has no license yet.

The bundled libmpv is an LGPL build: mpv with `-Dgpl=false`, FFmpeg without `--enable-gpl`. A GPL libmpv would make the whole app GPL. Third-party licenses are embedded and shown under Settings → About → Open-source licenses on both platforms.

Plaivo is not affiliated with Plex, Inc. or the Jellyfin project. Plaivo uses Trakt but is not endorsed or certified by Trakt.
