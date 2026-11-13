# Building Plaivo

How the app is built, packaged and released.

## Desktop app

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

`PLAIVO_LOG=debug` raises the log level.

## Installers and releases

```
cargo run -p xtask -- installer
```

It runs Velopack's `vpk pack` (install it once with `dotnet tool install -g vpk --version 1.2.161`) and writes everything to `target/velopack`: on Windows `Plaivo-win-Setup.exe`, a portable zip and the update packages; on macOS `Plaivo-osx-Setup.pkg`, a portable zip and the update packages.

Signing is optional:

| Variable | Effect |
|---|---|
| `PLAIVO_SIGN_CERT` | Windows: thumbprint of a code-signing certificate in your store; Velopack signs the app and the installer with it. |
| `PLAIVO_SIGN_IDENTITY` | macOS: a `Developer ID Application: …` identity; otherwise the app is ad-hoc signed. |
| `PLAIVO_SIGN_INSTALL_IDENTITY` | macOS: a `Developer ID Installer: …` identity; signs the .pkg. |
| `PLAIVO_NOTARY_PROFILE` | macOS: a `notarytool store-credentials` profile; notarizes the app and the .pkg. |

To release, run this from a clean, up-to-date `main` with the new version:

```
cargo run -p xtask -- release 0.2.0
```

It bumps `Cargo.toml`, commits, tags `v0.2.0`, asks before pushing, waits for the `release` workflow (which packs both platforms with delta updates from the previous release and uploads them to a draft GitHub release), shows the draft and asks before publishing it. The workflow reads the optional secrets `PLAIVO_TRAKT_ID`, `MACOS_CERT_P12` and `MACOS_CERT_PASSWORD` (a base64 `.p12` holding both the Developer ID Application and Developer ID Installer certificates), and `APPLE_ID`, `APPLE_TEAM_ID` and `APPLE_APP_PASSWORD` (notarization, required with the certificate). The CI Windows build is unsigned.

libmpv itself is built by `deps/build-libmpv.sh` and the `libmpv` workflow. Changing a pin needs a new `TAG`.

## Android app

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

## Tests

```
cargo test --workspace
```

Five tests that hit the network are `#[ignore]`d; run them with `cargo test --workspace -- --ignored`. The relay Worker's tests run with `bun test` in `workers/oauth-relay`.

## Layout

| Path | |
| --- | --- |
| `src/` | Desktop app: UI in `src/ui/`, mpv in `mpv.rs` and `player.rs`, the video window in `video_host.rs`. |
| `crates/core` | Shared core: Plex and Jellyfin clients, storage, settings, downloads, trackers, Watch Together. |
| `crates/ffi` | The UniFFI surface for Android. |
| `android/` | The Android and Android TV app. |
| `xtask/` | `libmpv`, `bundle`, `installer` (Velopack) and `release`; `resources/` holds the installer splash. |
| `resources/` | The app icon: `app.ico` (embedded in the Windows exe by `build.rs`) and `app.icns` (macOS bundle). |
| `deps/` | libmpv pins, build script, macOS embedding patch. |
| `shaders/` | mpv GLSL shaders (MIT, from upstream). |
| `licenses/` | Generates the in-app third-party license list. |
| `workers/` | `oauth-relay` (MyAnimeList sign-in) and `site` (plaivo.app), both Cloudflare Workers. |

## Licensing

Plaivo has no license yet. The bundled libmpv is an LGPL build (mpv `-Dgpl=false`, FFmpeg without `--enable-gpl`); a GPL libmpv would make the whole app GPL.
