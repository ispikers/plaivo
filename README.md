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

- A portable zip
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

### Installers

```
cargo run -p xtask -- installer
```

On Windows this builds an MSI, `target/release/Plaivo-<version>-x64.msi`, that installs for all users to `C:\Program Files\Plaivo` (with an admin prompt) and adds a Start menu shortcut. It needs the [WiX Toolset](https://wixtoolset.org) 7 CLI on `PATH` (`winget install WiXToolset.WiXCLI --version 7.0.1.0`). The build accepts WiX's [OSMF EULA](https://docs.firegiant.com/wix/osmf/) (`wix eula accept wix7`), which is free under US$10,000 a year in revenue; above that WiX asks for a maintenance fee. On macOS it builds `target/release/Plaivo-<version>-arm64.dmg` with the app and an Applications link. Both refuse to run until `deps/<os>` holds the pinned libmpv.

Signing is optional and driven by environment variables:

| Variable | Effect |
|---|---|
| `PLAIVO_SIGN_CERT` | Windows: SHA-1 thumbprint of a code-signing certificate in your store; `signtool` signs `plaivo.exe` and the MSI. |
| `PLAIVO_SIGN_IDENTITY` | macOS: a `Developer ID Application: …` identity; the app is signed with the hardened runtime, and so is the DMG. Without it the app is ad-hoc signed. |
| `PLAIVO_NOTARY_PROFILE` | macOS: a `notarytool store-credentials` keychain profile; the DMG is notarized and stapled. |

Unsigned builds work but trigger SmartScreen on Windows and Gatekeeper on macOS.

### Releases

Pushing a `v<version>` tag that matches `Cargo.toml` runs the `release` workflow: it builds the MSI and the DMG and attaches both to a draft GitHub release for you to publish. Running it by hand (`workflow_dispatch`) only uploads them as workflow artifacts. Repository secrets it reads, all optional:

| Secret | Used for |
|---|---|
| `PLAIVO_TRAKT_ID` | Baked into both builds (see Build-time variables). |
| `MACOS_CERT_P12`, `MACOS_CERT_PASSWORD` | A base64 `.p12` export of one Developer ID Application certificate and its password. With these the app and DMG are signed. |
| `APPLE_ID`, `APPLE_TEAM_ID`, `APPLE_APP_PASSWORD` | Notarization (an app-specific password). Required when the certificate is set. |

The Windows MSI from CI is unsigned. Code-signing keys now have to live on hardware or a cloud HSM, so sign locally with `PLAIVO_SIGN_CERT` (a token's certificate shows up in your store) or add your signing provider's step to the workflow.

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
| `xtask/` | `libmpv`, `bundle` and `installer` commands; `plaivo.wxs` is the Windows MSI source. |
| `deps/` | libmpv pins, build script and the macOS embedding patch. |
| `shaders/` | mpv GLSL shaders, MIT, from their upstream repositories. |
| `licenses/` | Generates the third-party license list shown in the app. |
| `workers/oauth-relay` | Cloudflare Worker for MyAnimeList sign-in. |
| `workers/site` | plaivo.app, a static site on Cloudflare Workers. |

## Licensing

Plaivo itself has no license yet.

The bundled libmpv is an LGPL build: mpv with `-Dgpl=false`, FFmpeg without `--enable-gpl`. A GPL libmpv would make the whole app GPL. Third-party licenses are embedded and shown under Settings → About → Open-source licenses on both platforms.

Plaivo is not affiliated with Plex, Inc. or the Jellyfin project. Plaivo uses Trakt but is not endorsed or certified by Trakt.
