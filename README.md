# Plaivo

A native Plex and Jellyfin client for Windows, macOS, Android and Android TV.

Video plays through mpv on desktop and ExoPlayer on Android, with hardware decoding and HDR. No browser engine, no web views.

## Download

There are no releases yet. When there are, they'll be on the [Releases](https://github.com/ispikers/plaivo/releases) page:

- **Windows 10 or 11 (x64):** `Plaivo-win-Setup.exe`. Installs in one click for your account, no admin prompt, and keeps itself up to date.
- **macOS 15 or later (Apple Silicon):** `Plaivo-osx-Setup.pkg`. Also updates itself.
- **Android 7.0 or later, including Android TV:** the APK.

## Features

- Plex and Jellyfin servers side by side in one sidebar.
- Direct play by default, with quality presets when your connection can't keep up.
- Downloads for offline viewing; watched progress syncs when you're back online.
- Skip intro and credits, an Up next countdown, and episode queues.
- Watchlist, trailers, and mark as watched or unwatched.
- Preferred audio and subtitle languages, synced with your account.
- Watch Together, compatible with Plezy rooms.
- Trakt and MyAnimeList scrobbling.
- Desktop: a miniplayer and anime/video upscaling shaders (Anime4K, ArtCNN, NVScaler).

Coming later: chapters and Plex Home user switching.

## Player keys

| Key | Action |
| --- | --- |
| Space, K | Play or pause (hold Space for 2× speed) |
| ← / → | Seek 10 s |
| Shift+← / → | Seek 30 s |
| ↑ / ↓ | Volume |
| M | Mute |
| F | Fullscreen |
| S | Skip intro or credits |
| N / P | Next or previous episode |
| Esc | Close a panel, leave fullscreen, then shrink to the miniplayer |

Seek lengths and what Esc does can be changed in Settings.

## Troubleshooting

Turn on Settings → Advanced → Debug logging, reproduce the problem, then open Settings → Advanced → Logs, press Export and attach `plaivo-log.txt` to an issue. Logs are kept in memory only, never written to disk until you export them. Sign-in tokens are left out of them.

| | Windows | macOS |
| --- | --- | --- |
| Exported log | `%APPDATA%\Plaivo\logs` | `~/Library/Caches/Plaivo/logs` |
| Settings | `%APPDATA%\Plaivo` | `~/Library/Application Support/Plaivo` |

An `mpv.conf` in the settings folder is passed to mpv.

Uninstalling keeps your sign-ins, settings and downloads, so reinstalling picks up where you left off. To remove them, use Settings → Advanced → Delete all data before uninstalling, or delete the settings folder above.

## Legal

Plaivo is released under the [MIT License](LICENSE). The libmpv it ships is LGPL-2.1-or-later, built from source by `deps/build-libmpv.sh`.

Third-party licenses are listed under Settings → About → Open-source licenses.

Plaivo is not affiliated with Plex, Inc. or the Jellyfin project. Plaivo uses Trakt but is not endorsed or certified by Trakt.
