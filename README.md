# Plaivo

A native Plex and Jellyfin client for Windows, macOS, Android and Android TV.

The desktop app is Rust: Zed's GPUI draws the interface and libmpv renders the video underneath it, so anything FFmpeg decodes plays the way mpv would play it, with hardware decoding and HDR passthrough. The Android app is Kotlin, Compose and ExoPlayer on the same Rust core, which handles the Plex and Jellyfin clients, settings, downloads, tracker sync and Watch Together. Neither uses a browser engine.

There are no releases yet.

## Features

- Plex (PIN sign-in, every server on the account) and Jellyfin (password or Quick Connect) side by side in one sidebar.
- Direct play by default; quality presets ask the server to transcode when the connection can't keep up.
- Downloads for offline viewing. Offline progress syncs to the server, Trakt and MyAnimeList once you're back online.
- Skip intro and credits, an Up next countdown, and the show's episodes as a queue.
- Mark as watched or unwatched, trailers, and the Plex watchlist.
- Preferred audio and subtitle languages, synced with your Plex or Jellyfin account.
- Watch Together over the openwatchparty protocol, so rooms are shared with Plezy users.
- Trakt and MyAnimeList scrobbling.
- Desktop only: a miniplayer, and mpv GLSL upscalers (Anime4K, ArtCNN, NVScaler).

Not done yet: an app icon, chapters, Plex Home user switching.

## Building the desktop app

Rust 1.85 or newer. First fetch the libmpv build pinned in `deps/libmpv.env`:

```
cargo run -p xtask -- libmpv
```

This downloads it from this repository's releases, checks its SHA-256 and unpacks it into `deps/windows/` or `deps/macos/`. Only Windows x64 and Apple Silicon Macs are supported. Then:

```
cargo build --release
```

`build.rs` copies libmpv and `shaders/` next to the executable. On macOS, run Plaivo as an app bundle (a bare binary opens inside Terminal):

```
cargo run -p xtask -- bundle --release --open
```

### Installers and releases

```
cargo run -p xtask -- installer
```

On Windows this builds `target/release/Plaivo-<version>-x64.msi` (per-machine install with a Start menu shortcut). It needs the [WiX Toolset](https://wixtoolset.org) 7 CLI (`winget install WiXToolset.WiXCLI --version 7.0.1.0`), and accepts WiX's [OSMF EULA](https://docs.firegiant.com/wix/osmf/) for you. On macOS it builds `target/release/Plaivo-<version>-arm64.dmg` (macOS 15 or later).

Signing is optional:

| Variable | Effect |
|---|---|
| `PLAIVO_SIGN_CERT` | Windows: thumbprint of a code-signing certificate in your store; signs the exe and MSI. |
| `PLAIVO_SIGN_IDENTITY` | macOS: a `Developer ID Application: …` identity; otherwise the app is ad-hoc signed. |
| `PLAIVO_NOTARY_PROFILE` | macOS: a `notarytool store-credentials` profile; notarizes and staples the DMG. |

Pushing a `v<version>` tag matching `Cargo.toml` runs the `release` workflow, which attaches the MSI and DMG to a draft GitHub release. It reads the optional secrets `PLAIVO_TRAKT_ID`, `MACOS_CERT_P12` and `MACOS_CERT_PASSWORD` (base64 Developer ID `.p12`), and `APPLE_ID`, `APPLE_TEAM_ID` and `APPLE_APP_PASSWORD` (notarization, required with the certificate). The CI MSI is unsigned; sign it locally.

libmpv itself is built by `deps/build-libmpv.sh` and the `libmpv` workflow. Changing a pin needs a new `TAG`.

## Building the Android app

One APK covers phones, tablets and Android TV, for Android 7.0+ on arm64 and armv7. You need the Android SDK, the NDK version in `android/app/build.gradle.kts`, JDK 17, and:

```
rustup target add aarch64-linux-android armv7-linux-androideabi
cargo install cargo-ndk
```

From `android/` (`./gradlew` outside Windows):

```
gradlew.bat assembleDebug
adb install -r app/build/outputs/apk/debug/app-debug.apk
```

`preBuild` compiles the Rust library and generates the Kotlin bindings, so the first build is slow. `assembleRelease` gives an unsigned APK to sign with `apksigner`. Format with `gradlew spotlessApply`, lint with `gradlew lintDebug`.

## Build-time variables

| Variable | Used for |
| --- | --- |
| `PLAIVO_TRAKT_ID` | The Trakt client ID. Without it, Trakt shows as "Not configured". |
| `PLAIVO_OAUTH_RELAY` | The MyAnimeList sign-in relay. Defaults to `https://relay.plaivo.app` (`workers/oauth-relay`). |

## Files and logs

| | Windows | macOS |
| --- | --- | --- |
| Account and settings | `%APPDATA%\Plaivo` | `~/Library/Application Support/Plaivo` |
| Cache | `%LOCALAPPDATA%\Plaivo\cache` | `~/Library/Caches/Plaivo/cache` |
| Logs | `%LOCALAPPDATA%\Plaivo\logs\plaivo.log` | `~/Library/Caches/Plaivo/logs/plaivo.log` |

`PLAIVO_LOG=debug` or Settings → Advanced → Debug logging gives verbose logs; tokens are redacted. An `mpv.conf` in the settings folder is read by mpv.

## Player keys

| Key | Action |
| --- | --- |
| Space, K | Play or pause (hold Space for 2× speed) |
| ← / → | Seek 10 s (configurable) |
| Shift+← / → | Seek 30 s (configurable) |
| ↑ / ↓ | Volume |
| M | Mute |
| F | Fullscreen |
| S | Skip intro or credits |
| N / P | Next or previous episode |
| Esc | Close a panel, leave fullscreen, then shrink to the miniplayer (or stop, per settings) |

## Tests

```
cargo test --workspace
```

Four tests that hit the network are `#[ignore]`d; run them with `cargo test --workspace -- --ignored`. The relay Worker's tests run with `bun test` in `workers/oauth-relay`.

## Layout

| Path | |
| --- | --- |
| `src/` | Desktop app: UI in `src/ui/`, mpv in `mpv.rs` and `player.rs`, the video window in `video_host.rs`. |
| `crates/core` | Shared core: Plex and Jellyfin clients, storage, settings, downloads, trackers, Watch Together. |
| `crates/ffi` | The UniFFI surface for Android. |
| `android/` | The Android and Android TV app. |
| `xtask/` | `libmpv`, `bundle` and `installer`; `plaivo.wxs` is the MSI source. |
| `deps/` | libmpv pins, build script, macOS embedding patch. |
| `shaders/` | mpv GLSL shaders (MIT, from upstream). |
| `licenses/` | Generates the in-app third-party license list. |
| `workers/` | `oauth-relay` (MyAnimeList sign-in) and `site` (plaivo.app), both Cloudflare Workers. |

## Licensing

Plaivo has no license yet.

The bundled libmpv is an LGPL build (mpv `-Dgpl=false`, FFmpeg without `--enable-gpl`); a GPL libmpv would make the whole app GPL. Third-party licenses are shown under Settings → About → Open-source licenses.

Plaivo is not affiliated with Plex, Inc. or the Jellyfin project. Plaivo uses Trakt but is not endorsed or certified by Trakt.
