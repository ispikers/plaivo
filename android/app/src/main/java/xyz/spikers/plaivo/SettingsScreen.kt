package xyz.spikers.plaivo

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import android.widget.Toast
import androidx.tv.material3.Border
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import coil3.SingletonImageLoader
import kotlinx.coroutines.launch
import kotlinx.coroutines.sync.Mutex
import kotlinx.coroutines.sync.withLock
import uniffi.plaivo_ffi.Section
import uniffi.plaivo_ffi.ServerInfo
import uniffi.plaivo_ffi.Settings
import uniffi.plaivo_ffi.Service
import uniffi.plaivo_ffi.SkipMode
import uniffi.plaivo_ffi.Status
import uniffi.plaivo_ffi.SubtitleMode
import uniffi.plaivo_ffi.Language
import uniffi.plaivo_ffi.LanguagePrefs
import uniffi.plaivo_ffi.languageName
import uniffi.plaivo_ffi.subtitleColors
import uniffi.plaivo_ffi.subtitleModeLabel
import uniffi.plaivo_ffi.languages
import uniffi.plaivo_ffi.moveLibrary

private enum class Category(val title: String, val glyph: Glyph, val parent: Category? = null) {
    General("General", Glyph.Settings),
    Libraries("Manage libraries", Glyph.Sort, General),
    Appearance("Appearance", Glyph.Palette),
    Playback("Playback", Glyph.PlayCircle),
    AudioSubtitles("Audio & Subtitles", Glyph.Subtitles),
    AudioLanguage("Audio language", Glyph.Subtitles, AudioSubtitles),
    SubtitleLanguage("Subtitle language", Glyph.Subtitles, AudioSubtitles),
    Services("Services", Glyph.Check),
    Downloads("Downloads", Glyph.Download),
    Connections("Connections", Glyph.Link),
    Advanced("Advanced", Glyph.Build),
    About("About", Glyph.Info),
}

private val COUNTDOWNS = listOf(0u, 3u, 5u, 10u, 15u, 20u, 30u)
private val SEEK_SMALL = listOf(5u, 10u, 15u, 20u, 30u)
private val SEEK_LARGE = listOf(30u, 60u, 90u, 120u, 300u)
private val SUBTITLE_SIZES = listOf(50u, 75u, 100u, 125u, 150u, 175u, 200u)
private val SUBTITLE_OUTLINES = listOf(0u, 3u)
private val SUBTITLE_BACKGROUNDS = listOf(0u, 25u, 50u, 75u, 100u)
private val SUBTITLE_POSITIONS = (50u..100u step 5).reversed().toList()
private val SUBTITLE_COLORS = subtitleColors()
private val accountWrites = Mutex()
private val DENSITIES = listOf("Smallest", "Small", "Medium", "Large", "Largest")

private fun <T> List<T>.after(value: T) = this[(indexOf(value) + 1) % size]

private fun densityLabel(density: UInt) = DENSITIES[density.toInt().coerceIn(1, DENSITIES.size) - 1]

private val SkipMode.label get() = name.lowercase().replaceFirstChar { it.uppercase() }

private val SubtitleMode.label get() = subtitleModeLabel(this)

private val Settings.languages get() = LanguagePrefs(audioLanguage, subtitleLanguage, subtitleMode)

private fun offOr(value: UInt, text: String) = if (value == 0u) "Off" else text

private fun onOff(on: Boolean) = if (on) "on" else "off"

private val Status.label get() = if (!configured) "Not configured" else if (signedIn) "Signed in" else "Not signed in"

@Composable
fun SettingsScreen(
    servers: List<ServerInfo>,
    home: ServerInfo?,
    settings: Settings,
    libraries: List<Section>,
    onEdit: ((Settings) -> Settings) -> Unit,
    onSignOut: () -> Unit,
    onPlexSignIn: () -> Unit,
    onServersChanged: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val version = remember { runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: "" }
    val qualities = remember { core.qualities() }
    val quality = settings.quality.toInt()
    val qualityLabel = qualities.firstOrNull { it.kbps.toInt() == quality }?.let { "${it.label} · ${it.bitrate}" } ?: "Original"
    var category by rememberSaveable { mutableStateOf<Category?>(null) }
    var last by rememberSaveable { mutableStateOf(Category.General) }
    var navigated by remember { mutableStateOf(false) }
    var focused by remember { mutableStateOf(false) }
    var cacheCleared by remember { mutableStateOf(false) }
    val first = remember { FocusRequester() }
    var fromLicenses by rememberSaveable { mutableStateOf(false) }
    fun open(c: Category?) {
        navigated = true
        fromLicenses = false
        category = c
    }
    var jellyfin by remember { mutableStateOf(core.jellyfinServers()) }
    val addServerFocus = remember { FocusRequester() }
    var adding by rememberSaveable { mutableStateOf(false) }
    if (adding) {
        AddServerScreen(onBack = { adding = false }) {
            adding = false
            jellyfin = core.jellyfinServers()
            onServersChanged()
        }
        return
    }
    var trackerTick by remember { mutableIntStateOf(0) }
    val trackers = remember(trackerTick) { Service.entries.associateWith { core.trackerStatus(it) } }
    val traktProgress = remember(trackerTick) { core.traktProgress() }
    var lastService by rememberSaveable { mutableStateOf(Service.TRAKT) }
    var connecting by rememberSaveable { mutableStateOf<Service?>(null) }
    connecting?.let {
        TrackerSignInScreen(it) {
            connecting = null
            trackerTick++
        }
        return
    }
    var licenses by rememberSaveable { mutableStateOf(false) }
    if (licenses) {
        LicensesScreen {
            licenses = false
            fromLicenses = true
        }
        return
    }
    BackHandler(enabled = category != null && focused) { open(category?.parent) }
    LaunchedEffect(category) { if (navigated) runCatching { first.requestFocus() } }
    fun Modifier.takeFocus(take: Boolean) = if (take) focusRequester(first) else this
    fun syncLanguages() = scope.launch {
        val before = settings.languages
        val p = accountWrites.withLock { io { runCatching { core.accountLanguages() }.getOrNull() } } ?: return@launch
        onEdit { if (it.languages == before) it.copy(audioLanguage = p.audio, subtitleLanguage = p.subtitles, subtitleMode = p.mode) else it }
    }
    fun editLanguages(change: (LanguagePrefs) -> LanguagePrefs) {
        val p = change(settings.languages)
        onEdit { it.copy(audioLanguage = p.audio, subtitleLanguage = p.subtitles, subtitleMode = p.mode) }
        scope.launch {
            accountWrites.withLock { io { runCatching { core.setAccountLanguages(p) } } }.onFailure { Toast.makeText(context, "Could not save to your account: ${it.message}", Toast.LENGTH_LONG).show() }
        }
    }
    key(category) {
        Column(
            Modifier.fillMaxSize().onFocusChanged { focused = it.hasFocus }.verticalScroll(rememberScrollState()).padding(PaddingValues(horizontal = Dims.edge, vertical = 28.dp)),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Heading(category?.title ?: "Settings")
            when (category) {
                null -> Category.entries.filter { it.parent == null }.forEach { c ->
                    val summary = when (c) {
                        Category.General -> home?.let { "Home server: ${it.name}" } ?: "No home server"
                        Category.Libraries, Category.AudioLanguage, Category.SubtitleLanguage -> ""
                        Category.AudioSubtitles -> "Audio: ${languageName(settings.audioLanguage)} · Subtitles: ${settings.subtitleMode.label}"
                        Category.Appearance -> "Poster size: ${densityLabel(settings.density)}"
                        Category.Playback -> "Default quality: $qualityLabel · Autoplay ${onOff(settings.autoplayNext)}"
                        Category.Services -> Service.entries.joinToString(" · ") { "${it.title}: ${trackers.getValue(it).label}" }
                        Category.Downloads -> "Auto-remove watched ${onOff(settings.autoRemoveWatched)}"
                        Category.Connections -> {
                            fun count(n: Int) = if (n == 1) "1 server" else "$n servers"
                            val plex = if (core.signedIn()) count(servers.count { !it.jellyfin }) else "Not signed in"
                            "Plex: $plex · Jellyfin: ${count(jellyfin.size)}"
                        }
                        Category.Advanced -> "Image cache · Debug logging ${onOff(settings.debugLogging)}"
                        Category.About -> "Version $version"
                    }
                    SettingsRow(c.glyph, c.title, summary, modifier = Modifier.takeFocus(c == last), onClick = {
                        last = c
                        if (c == Category.AudioSubtitles) syncLanguages()
                        open(c)
                    }) { GlyphIcon(Glyph.ChevronRight, 22.dp, Palette.muted) }
                }
                Category.General -> {
                    SectionTitle("Home server")
                    Note("Home shows Continue Watching and recommendations from this server. Libraries from every server stay in the sidebar.")
                    servers.forEach { s ->
                        val selected = s.id == home?.id
                        SettingsRow(Glyph.Server, s.name, if (s.jellyfin) "Jellyfin" else if (s.owned) "Owned" else "Shared with you", Modifier.takeFocus(selected), selected, { onEdit { it.copy(homeServer = s.id) } }) {
                            if (selected) GlyphIcon(Glyph.Check, 20.dp, Palette.text)
                        }
                    }
                    if (servers.isEmpty()) SettingsRow(Glyph.Server, "No servers found", "Add Plex or Jellyfin servers under Connections", Modifier.takeFocus(true))
                    SectionTitle("Sidebar")
                    SettingsRow(Glyph.Sort, "Manage libraries", "Reorder and hide the libraries in the sidebar", onClick = { open(Category.Libraries) }) { GlyphIcon(Glyph.ChevronRight, 22.dp, Palette.muted) }
                }
                Category.Libraries -> {
                    Note("Order and visibility are saved for each library on each server.")
                    val ids = libraries.map { it.id }
                    libraries.groupBy { it.serverName }.forEach { (server, libs) ->
                        SectionTitle(server)
                        libs.forEachIndexed { i, lib ->
                            key(lib.id) {
                                val hidden = lib.id in settings.hiddenLibraries
                                Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                                    SettingsRow(if (lib.movies) Glyph.Movie else Glyph.Tv, lib.title, if (hidden) "Hidden" else "Shown", Modifier.weight(1f).takeFocus(lib.id == ids.firstOrNull()), onClick = {
                                        onEdit { it.copy(hiddenLibraries = if (hidden) it.hiddenLibraries - lib.id else it.hiddenLibraries + lib.id) }
                                    }) { Switch(!hidden) }
                                    MoveButton(Glyph.ExpandLess, i > 0) { onEdit { moveLibrary(it, ids, lib.id, -1) } }
                                    MoveButton(Glyph.ExpandMore, i < libs.lastIndex) { onEdit { moveLibrary(it, ids, lib.id, 1) } }
                                }
                            }
                        }
                    }
                    if (libraries.isEmpty()) SettingsRow(Glyph.Sort, "No libraries yet", "Libraries appear here once your servers load", Modifier.takeFocus(true))
                }
                Category.Appearance -> {
                    SectionTitle("Library")
                    SettingsRow(Glyph.Movie, "Poster size", "Size of posters on Home, in libraries and in search", Modifier.takeFocus(true), onClick = {
                        onEdit { it.copy(density = (1u..DENSITIES.size.toUInt()).toList().after(it.density)) }
                    }) { Value(densityLabel(settings.density)) }
                }
                Category.Playback -> {
                    SectionTitle("Up next")
                    SettingsRow(Glyph.PlayCircle, "Autoplay next episode", "Plays the next episode in the queue when one ends", Modifier.takeFocus(true), onClick = {
                        onEdit { it.copy(autoplayNext = !it.autoplayNext) }
                    }) { Switch(settings.autoplayNext) }
                    if (settings.autoplayNext) {
                        SettingsRow(null, "Countdown", "Seconds the Up next card waits before playing. 0 plays at once.", onClick = {
                            onEdit { it.copy(autoplayCountdown = COUNTDOWNS.after(it.autoplayCountdown)) }
                        }) { Value("${settings.autoplayCountdown} s") }
                    }
                    SectionTitle("Skip markers")
                    Note("Button shows a skip button. Auto skips on its own.")
                    SettingsRow(null, "Intro", "Opening titles and recaps", onClick = { onEdit { it.copy(skipIntro = SkipMode.entries.after(it.skipIntro)) } }) { Value(settings.skipIntro.label) }
                    SettingsRow(null, "Credits", "End credits", onClick = { onEdit { it.copy(skipCredits = SkipMode.entries.after(it.skipCredits)) } }) { Value(settings.skipCredits.label) }
                    SectionTitle("Seeking")
                    SettingsRow(null, "Small step", "Left and right on the remote", onClick = { onEdit { it.copy(seekSmall = SEEK_SMALL.after(it.seekSmall)) } }) { Value("${settings.seekSmall} s") }
                    SettingsRow(null, "Large step", "Holding left or right, and fast-forward or rewind", onClick = { onEdit { it.copy(seekLarge = SEEK_LARGE.after(it.seekLarge)) } }) { Value("${settings.seekLarge} s") }
                    SectionTitle("Default quality")
                    Note("Videos play at this quality unless the file is already smaller. Changing quality in the player updates it too.")
                    val original = qualities.none { it.kbps.toInt() == quality }
                    SettingsRow(null, "Original", "Direct play, no transcoding", selected = original, onClick = { onEdit { it.copy(quality = 0u) } }) {
                        if (original) GlyphIcon(Glyph.Check, 20.dp, Palette.text)
                    }
                    qualities.forEach { q ->
                        val selected = q.kbps.toInt() == quality
                        SettingsRow(null, q.label, q.bitrate, selected = selected, onClick = { onEdit { it.copy(quality = q.kbps) } }) {
                            if (selected) GlyphIcon(Glyph.Check, 20.dp, Palette.text)
                        }
                    }
                }
                Category.AudioSubtitles -> {
                    SectionTitle("Languages")
                    Note("Saved to your Plex account and Jellyfin users, so other apps follow them too. This device keeps a copy for offline playback.")
                    SettingsRow(null, "Audio language", languageName(settings.audioLanguage), Modifier.takeFocus(true), onClick = { open(Category.AudioLanguage) }) { GlyphIcon(Glyph.ChevronRight, 22.dp, Palette.muted) }
                    SettingsRow(null, "Subtitle language", languageName(settings.subtitleLanguage), onClick = { open(Category.SubtitleLanguage) }) { GlyphIcon(Glyph.ChevronRight, 22.dp, Palette.muted) }
                    SettingsRow(null, "Show subtitles", "Forced subtitles for foreign dialogue show in every mode", onClick = {
                        editLanguages { it.copy(mode = SubtitleMode.entries.after(it.mode)) }
                    }) { Value(settings.subtitleMode.label) }
                    SectionTitle("Subtitle style")
                    Note("Applies to text subtitles. Styled ASS subtitles keep their own colours.")
                    SettingsRow(null, "Size", "Text size", onClick = { onEdit { it.copy(subtitleSize = SUBTITLE_SIZES.after(it.subtitleSize)) } }) { Value("${settings.subtitleSize}%") }
                    SettingsRow(null, "Colour", "Text colour", onClick = {
                        onEdit { it.copy(subtitleColor = SUBTITLE_COLORS.map { c -> c.hex }.after(it.subtitleColor.uppercase())) }
                    }) { Value(SUBTITLE_COLORS.firstOrNull { it.hex.equals(settings.subtitleColor, ignoreCase = true) }?.name ?: settings.subtitleColor) }
                    SettingsRow(null, "Outline", "Black outline around the text", onClick = { onEdit { it.copy(subtitleOutline = SUBTITLE_OUTLINES.after(it.subtitleOutline)) } }) {
                        Value(offOr(settings.subtitleOutline, "On"))
                    }
                    SettingsRow(null, "Background", "Opacity of the box behind the text", onClick = { onEdit { it.copy(subtitleBackground = SUBTITLE_BACKGROUNDS.after(it.subtitleBackground)) } }) {
                        Value(offOr(settings.subtitleBackground, "${settings.subtitleBackground}%"))
                    }
                    SettingsRow(null, "Position", "Height above the bottom of the video", onClick = { onEdit { it.copy(subtitlePosition = SUBTITLE_POSITIONS.after(it.subtitlePosition)) } }) {
                        val lift = 100u - settings.subtitlePosition.coerceAtMost(100u)
                        Value(if (lift == 0u) "Bottom" else "$lift% up")
                    }
                }
                Category.AudioLanguage, Category.SubtitleLanguage -> {
                    val subtitles = category == Category.SubtitleLanguage
                    val all = remember { listOf(Language("", "Any")) + languages() }
                    val current = if (subtitles) settings.subtitleLanguage else settings.audioLanguage
                    val focus = if (all.any { it.code == current }) current else ""
                    all.forEach { l ->
                        val selected = l.code == current
                        SettingsRow(null, l.name, "", Modifier.takeFocus(l.code == focus), selected, {
                            editLanguages { if (subtitles) it.copy(subtitles = l.code) else it.copy(audio = l.code) }
                        }) { if (selected) GlyphIcon(Glyph.Check, 20.dp, Palette.text) }
                    }
                }
                Category.Services -> {
                    Note("Marks what you watch as watched on these services. Trakt also shows what you are watching now.")
                    Service.entries.forEach { s ->
                        val status = trackers.getValue(s)
                        SectionTitle(s.title)
                        if (!status.configured) {
                            SettingsRow(Glyph.Person, s.title, "Not configured in this build", Modifier.takeFocus(s == lastService))
                            return@forEach
                        }
                        SettingsRow(Glyph.Person, "Account", status.label, Modifier.takeFocus(s == lastService), onClick = {
                            lastService = s
                            if (status.signedIn) {
                                core.trackerSignOut(s)
                                trackerTick++
                            } else {
                                connecting = s
                            }
                        }) { Value(if (status.signedIn) "Disconnect" else "Connect") }
                        SettingsRow(Glyph.Check, "Scrobble", "Send what you watch to ${s.title}", onClick = {
                            core.setTrackerScrobble(s, !status.scrobble)
                            trackerTick++
                        }) { Switch(status.scrobble) }
                        if (s == Service.TRAKT) {
                            SettingsRow(Glyph.PlayCircle, "Scrobble progress", "Shows what you are watching now. Off sends a play only once you finish.", onClick = {
                                core.setTraktProgress(!traktProgress)
                                trackerTick++
                            }) { Switch(traktProgress) }
                        }
                    }
                }
                Category.Downloads -> {
                    SectionTitle("Location")
                    Note("Downloads are kept in Plaivo's own storage on this device. Uninstalling Plaivo deletes them.")
                    val root = DownloadQueue.root
                    SettingsRow(Glyph.Download, "Download folder", root.absolutePath, Modifier.takeFocus(true)) {
                        Value("${fmtSize(root.usableSpace.toULong())} free")
                    }
                    SectionTitle("Cleanup")
                    SettingsRow(Glyph.Check, "Auto-remove watched", "Deletes a download once you finish watching it", onClick = {
                        onEdit { it.copy(autoRemoveWatched = !it.autoRemoveWatched) }
                    }) { Switch(settings.autoRemoveWatched) }
                }
                Category.Connections -> {
                    SectionTitle("Plex")
                    if (core.signedIn()) {
                        SettingsRow(Glyph.Person, "Plex account", "Signed in", Modifier.takeFocus(true))
                        SettingsRow(Glyph.Logout, "Sign out", "Removes this device's Plex sign-in", onClick = onSignOut)
                    } else {
                        SettingsRow(Glyph.Person, "Sign in with Plex", "Link this device at plex.tv/link", Modifier.takeFocus(true), onClick = onPlexSignIn)
                    }
                    SectionTitle("Jellyfin")
                    jellyfin.forEach { s ->
                        key(s.id) {
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
                                SettingsRow(Glyph.Server, s.name, listOf(s.user, s.address).filter { it.isNotEmpty() }.joinToString(" · "), Modifier.weight(1f))
                                MoveButton(Glyph.Close, true) {
                                    core.removeJellyfin(s.id)
                                    jellyfin = core.jellyfinServers()
                                    onServersChanged()
                                    runCatching { addServerFocus.requestFocus() }
                                }
                            }
                        }
                    }
                    SettingsRow(Glyph.Add, "Add server", "Sign in with a username and password, or Quick Connect", Modifier.focusRequester(addServerFocus), onClick = { adding = true }) {
                        GlyphIcon(Glyph.ChevronRight, 22.dp, Palette.muted)
                    }
                }
                Category.Advanced -> {
                    SectionTitle("Storage")
                    SettingsRow(Glyph.Refresh, "Clear image cache", if (cacheCleared) "Cleared" else "Removes cached artwork and library data", Modifier.takeFocus(true), onClick = {
                        val images = SingletonImageLoader.get(context)
                        images.memoryCache?.clear()
                        scope.launch {
                            io {
                                images.diskCache?.clear()
                                core.clearCache()
                            }
                            cacheCleared = true
                        }
                    })
                    SectionTitle("Logging")
                    SettingsRow(Glyph.Info, "Debug logging", "More detail in logcat. Turn it on when reporting a problem.", onClick = {
                        onEdit { it.copy(debugLogging = !it.debugLogging) }
                    }) { Switch(settings.debugLogging) }
                }
                Category.About -> {
                    SectionTitle("Plaivo")
                    SettingsRow(Glyph.PlayCircle, "Plaivo", "Version $version", modifier = Modifier.takeFocus(!fromLicenses))
                    SettingsRow(Glyph.Info, "Open-source licenses", "Kotlin, AndroidX, Media3 and the Rust crates", Modifier.takeFocus(fromLicenses), onClick = { licenses = true }) {
                        GlyphIcon(Glyph.ChevronRight, 22.dp, Palette.muted)
                    }
                }
            }
        }
    }
}

@Composable
private fun Value(text: String) {
    Text(text, fontSize = 14.sp, color = Palette.muted)
}

@Composable
private fun Switch(on: Boolean) {
    Box(Modifier.width(40.dp).height(24.dp).background(if (on) Palette.text else Palette.tint(0.2f), CircleShape).padding(3.dp), contentAlignment = if (on) Alignment.CenterEnd else Alignment.CenterStart) {
        Box(Modifier.size(18.dp).background(if (on) Palette.bg else Palette.text, CircleShape))
    }
}

@Composable
fun MoveButton(glyph: Glyph, enabled: Boolean, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        enabled = enabled,
        modifier = Modifier.size(44.dp).alpha(if (enabled) 1f else 0.35f),
        shape = ClickableSurfaceDefaults.shape(CircleShape),
        colors = ClickableSurfaceDefaults.colors(containerColor = Palette.surface, contentColor = Palette.text, focusedContainerColor = Palette.tint(0.12f), focusedContentColor = Palette.text, disabledContainerColor = Palette.surface),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
        border = ClickableSurfaceDefaults.border(focusedBorder = Border(BorderStroke(2.dp, Palette.text), shape = CircleShape)),
    ) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { GlyphIcon(glyph, 24.dp, Palette.text) }
    }
}

@Composable
private fun SectionTitle(text: String) {
    Text(text, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = Palette.muted, modifier = Modifier.padding(top = 8.dp))
}

@Composable
private fun Note(text: String) {
    Text(text, fontSize = 13.sp, color = Palette.muted)
}

@Composable
fun SettingsRow(glyph: Glyph?, title: String, detail: String, modifier: Modifier = Modifier, selected: Boolean = false, onClick: () -> Unit = {}, trailing: @Composable () -> Unit = {}) {
    val shape = RoundedCornerShape(12.dp)
    Surface(
        onClick = onClick,
        modifier = modifier.fillMaxWidth(),
        shape = ClickableSurfaceDefaults.shape(shape),
        colors = ClickableSurfaceDefaults.colors(
            containerColor = if (selected) Palette.tint(0.10f) else Palette.surface,
            contentColor = Palette.text,
            focusedContainerColor = Palette.tint(0.12f),
            focusedContentColor = Palette.text,
        ),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
        border = ClickableSurfaceDefaults.border(focusedBorder = Border(BorderStroke(2.dp, Palette.text), shape = shape)),
    ) {
        Row(Modifier.fillMaxWidth().height(58.dp).padding(horizontal = 16.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(14.dp)) {
            if (glyph != null) GlyphIcon(glyph, 22.dp, Palette.muted)
            Column(Modifier.weight(1f)) {
                Text(title, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = Palette.text, maxLines = 1)
                Text(detail, fontSize = 12.sp, color = Palette.muted, maxLines = 1)
            }
            trailing()
        }
    }
}
