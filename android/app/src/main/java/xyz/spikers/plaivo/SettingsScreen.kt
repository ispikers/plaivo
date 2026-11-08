package xyz.spikers.plaivo

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Border
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import uniffi.plaivo_ffi.ServerInfo

private enum class Category(val title: String, val glyph: Glyph) {
    General("General", Glyph.Settings),
    Playback("Playback", Glyph.PlayCircle),
    Connections("Connections", Glyph.Link),
    About("About", Glyph.Info),
}

@Composable
fun SettingsScreen(servers: List<ServerInfo>, home: ServerInfo?, quality: Int, onHome: (String) -> Unit, onQuality: (Int) -> Unit, onSignOut: () -> Unit) {
    val context = LocalContext.current
    val version = remember { runCatching { context.packageManager.getPackageInfo(context.packageName, 0).versionName }.getOrNull() ?: "" }
    val qualities = remember { core.qualities() }
    val qualityLabel = qualities.firstOrNull { it.kbps.toInt() == quality }?.let { "${it.label} · ${it.bitrate}" } ?: "Original"
    var category by rememberSaveable { mutableStateOf<Category?>(null) }
    var last by rememberSaveable { mutableStateOf(Category.General) }
    var navigated by remember { mutableStateOf(false) }
    var focused by remember { mutableStateOf(false) }
    val first = remember { FocusRequester() }
    fun open(c: Category?) {
        navigated = true
        category = c
    }
    BackHandler(enabled = category != null && focused) { open(null) }
    LaunchedEffect(category) { if (navigated) runCatching { first.requestFocus() } }
    fun Modifier.takeFocus(take: Boolean) = if (take) focusRequester(first) else this
    key(category) {
        Column(
            Modifier.fillMaxSize().onFocusChanged { focused = it.hasFocus }.verticalScroll(rememberScrollState()).padding(PaddingValues(horizontal = Dims.edge, vertical = 28.dp)),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Heading(category?.title ?: "Settings")
            when (category) {
                null -> Category.entries.forEach { c ->
                    val summary = when (c) {
                        Category.General -> home?.let { "Home server: ${it.name}" } ?: "No home server"
                        Category.Playback -> "Default quality: $qualityLabel"
                        Category.Connections -> if (servers.size == 1) "Plex account · 1 server" else "Plex account · ${servers.size} servers"
                        Category.About -> "Version $version"
                    }
                    SettingsRow(c.glyph, c.title, summary, modifier = Modifier.takeFocus(c == last), onClick = {
                        last = c
                        open(c)
                    }) { GlyphIcon(Glyph.ChevronRight, 22.dp, Palette.muted) }
                }
                Category.General -> {
                    SectionTitle("Home server")
                    Note("Home shows Continue Watching and recommendations from this server. Libraries from every server stay in the sidebar.")
                    servers.forEach { s ->
                        val selected = s.id == home?.id
                        SettingsRow(Glyph.Server, s.name, if (s.owned) "Owned" else "Shared with you", Modifier.takeFocus(selected), selected, { onHome(s.id) }) {
                            if (selected) GlyphIcon(Glyph.Check, 20.dp, Palette.text)
                        }
                    }
                    if (servers.isEmpty()) SettingsRow(Glyph.Server, "No servers found", "Servers on your Plex account appear here", Modifier.takeFocus(true))
                }
                Category.Playback -> {
                    SectionTitle("Default quality")
                    Note("Videos play at this quality unless the file is already smaller. Changing quality in the player updates it too.")
                    val original = qualities.none { it.kbps.toInt() == quality }
                    SettingsRow(null, "Original", "Direct play, no transcoding", Modifier.takeFocus(original), original, { onQuality(0) }) {
                        if (original) GlyphIcon(Glyph.Check, 20.dp, Palette.text)
                    }
                    qualities.forEach { q ->
                        val selected = q.kbps.toInt() == quality
                        SettingsRow(null, q.label, q.bitrate, Modifier.takeFocus(selected), selected, { onQuality(q.kbps.toInt()) }) {
                            if (selected) GlyphIcon(Glyph.Check, 20.dp, Palette.text)
                        }
                    }
                }
                Category.Connections -> {
                    SectionTitle("Plex")
                    SettingsRow(Glyph.Person, "Plex account", "Signed in", Modifier.takeFocus(true))
                    SettingsRow(Glyph.Logout, "Sign out", "Removes this device's Plex sign-in", onClick = onSignOut)
                }
                Category.About -> {
                    SectionTitle("Plaivo")
                    SettingsRow(Glyph.PlayCircle, "Plaivo", "Version $version", modifier = Modifier.takeFocus(true))
                }
            }
        }
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
private fun SettingsRow(glyph: Glyph?, title: String, detail: String, modifier: Modifier = Modifier, selected: Boolean = false, onClick: () -> Unit = {}, trailing: @Composable () -> Unit = {}) {
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
