package xyz.spikers.plaivo

import android.content.Intent
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.tv.material3.Border
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import kotlinx.coroutines.launch
import uniffi.plaivo_ffi.Action
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.DownloadScope
import uniffi.plaivo_ffi.ItemMenu
import uniffi.plaivo_ffi.actionLabel

private enum class MenuPage {
    Main,
    Versions,
    Info,
    Rate,
}

private val CONFIRM_KEYS = setOf(Key.DirectionCenter, Key.Enter, Key.NumPadEnter)

private val Action.glyph
    get() =
        when (this) {
            Action.FROM_START -> Glyph.Replay
            Action.MARK_WATCHED -> Glyph.Check
            Action.MARK_UNWATCHED -> Glyph.Remove
            Action.CLEAR_PROGRESS -> Glyph.Close
            Action.TRAILER -> Glyph.Movie
            Action.RATE -> Glyph.StarBorder
            Action.VERSIONS -> Glyph.Layers
            Action.FILE_INFO -> Glyph.HighQuality
            Action.DOWNLOAD -> Glyph.Download
            Action.ADD_TO_WATCHLIST -> Glyph.BookmarkAdd
            Action.REMOVE_FROM_WATCHLIST -> Glyph.Bookmark
            Action.DETAILS -> Glyph.Info
        }

private fun downloadScopes(kind: String) =
    when (kind) {
        "movie" -> listOf("Download movie" to DownloadScope.ITEM)
        "episode" ->
            listOf(
                "Download episode" to DownloadScope.ITEM,
                "Download season" to DownloadScope.SEASON,
                "Download show" to DownloadScope.SHOW,
            )
        "season" -> listOf("Download season" to DownloadScope.ITEM, "Download show" to DownloadScope.SHOW)
        "show" -> listOf("Download show" to DownloadScope.ITEM)
        else -> emptyList()
    }

@Composable
fun CardMenu(card: MediaCard, onDismiss: () -> Unit, onChanged: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val entries by DownloadQueue.entries.collectAsState()
    val saved = entries.any { it.server == card.server && it.key == card.key }
    var menu by remember { mutableStateOf<ItemMenu?>(null) }
    var failed by remember { mutableStateOf<String?>(null) }
    var page by remember { mutableStateOf(MenuPage.Main) }
    var pressed by remember { mutableStateOf(false) }
    val first = remember { FocusRequester() }
    LaunchedEffect(card) {
        io { runCatching { core.itemMenu(card.server, card.key) } }
            .onSuccess { menu = it }
            .onFailure { failed = it.message }
    }
    fun player(key: String = card.key, extras: Intent.() -> Unit = {}) {
        context.startActivity(
            Intent(context, PlayerActivity::class.java)
                .putExtra("server", card.server)
                .putExtra("key", key)
                .apply(extras)
        )
        onDismiss()
    }
    fun run(action: Action) =
        scope.launch {
            val result = io { runCatching { core.menuAction(card.server, card.key, action) } }
            Toast.makeText(context, result.getOrElse { "Could not update: ${it.message}" }, Toast.LENGTH_SHORT).show()
            if (result.isSuccess) onChanged()
            onDismiss()
        }
    fun rate(stars: UByte?) =
        scope.launch {
            val result = io { runCatching { core.rate(card.server, card.key, stars) } }
            Toast.makeText(
                    context,
                    result.fold(
                        { stars?.let { "Rated $it of 5" } ?: "Rating cleared" },
                        { "Could not rate: ${it.message}" },
                    ),
                    Toast.LENGTH_SHORT,
                )
                .show()
            onDismiss()
        }
    val shape = RoundedCornerShape(16.dp)
    val touch = LocalTouch.current
    MenuFrame(page != MenuPage.Main, { page = MenuPage.Main }, onDismiss) {
        Column(
            if (touch) Modifier.verticalScroll(rememberScrollState()).padding(Dims.gutter, 0.dp, Dims.gutter, 16.dp)
            else
                Modifier.width(460.dp)
                    .background(Palette.bg, shape)
                    .border(1.dp, Palette.outline, shape)
                    .padding(20.dp)
                    .onPreviewKeyEvent { e ->
                        if (e.key !in CONFIRM_KEYS) return@onPreviewKeyEvent false
                        when (e.type) {
                            KeyEventType.KeyDown ->
                                if (e.nativeKeyEvent.repeatCount == 0) {
                                    pressed = true
                                    false
                                } else !pressed
                            KeyEventType.KeyUp -> !pressed
                            else -> false
                        }
                    },
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(card.title, fontSize = 18.sp, fontWeight = FontWeight.Bold, color = Palette.text, maxLines = 1)
            if (card.subtitle.isNotEmpty()) Text(card.subtitle, fontSize = 13.sp, color = Palette.muted, maxLines = 1)
            val m = menu
            when (page) {
                MenuPage.Main -> {
                    var focus = true
                    fun mod() = if (focus) Modifier.focusRequester(first).also { focus = false } else Modifier
                    if (m == null)
                        Text(
                            failed?.let { "Could not load: $it" } ?: "Loading…",
                            fontSize = 14.sp,
                            color = Palette.muted,
                        )
                    m?.actions?.forEach { a ->
                        when (a) {
                            Action.DETAILS -> Unit
                            Action.DOWNLOAD ->
                                if (saved) {
                                    SettingsRow(
                                        Glyph.Close,
                                        "Remove download",
                                        "Deletes it from this device",
                                        mod(),
                                        onClick = {
                                            background.launch { core.removeDownload(card.server, card.key) }
                                            onDismiss()
                                        },
                                    )
                                } else {
                                    downloadScopes(card.kind).forEach { (label, s) ->
                                        SettingsRow(
                                            Glyph.Download,
                                            label,
                                            "",
                                            mod(),
                                            onClick = {
                                                download(context, card, s)
                                                onDismiss()
                                            },
                                        )
                                    }
                                }
                            else ->
                                SettingsRow(
                                    a.glyph,
                                    actionLabel(a),
                                    "",
                                    mod(),
                                    onClick = {
                                        when (a) {
                                            Action.FROM_START -> player { putExtra("fromStart", true) }
                                            Action.TRAILER -> m.trailerKey?.let { player(it) }
                                            Action.RATE -> page = MenuPage.Rate
                                            Action.VERSIONS -> page = MenuPage.Versions
                                            Action.FILE_INFO -> page = MenuPage.Info
                                            else -> run(a)
                                        }
                                    },
                                )
                        }
                    }
                    if (downloadScopes(card.kind).isNotEmpty() && (!Watch.inRoom || Watch.state?.host == true)) {
                        SettingsRow(
                            Glyph.Group,
                            "Watch together",
                            if (Watch.inRoom) "Plays it for everyone in the room"
                            else "Starts a room and plays it there",
                            mod(),
                            onClick = {
                                Watch.share(card)
                                onDismiss()
                            },
                        )
                    }
                }
                MenuPage.Versions ->
                    m?.versions?.forEachIndexed { i, label ->
                        SettingsRow(
                            Glyph.Layers,
                            "Version ${i + 1}",
                            label,
                            if (i == 0) Modifier.focusRequester(first) else Modifier,
                            onClick = { player { putExtra("version", i) } },
                        )
                    }
                MenuPage.Info -> {
                    m?.fileInfo?.forEach { line ->
                        Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
                            Text(
                                line.label,
                                fontSize = 13.sp,
                                fontWeight = FontWeight.SemiBold,
                                color = Palette.muted,
                                modifier = Modifier.width(96.dp),
                            )
                            Text(line.value, fontSize = 13.sp, color = Palette.text)
                        }
                    }
                    SettingsRow(
                        Glyph.ChevronLeft,
                        "Back",
                        "",
                        Modifier.focusRequester(first),
                        onClick = { page = MenuPage.Main },
                    )
                }
                MenuPage.Rate -> {
                    val current = m?.stars?.toInt() ?: 0
                    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        (1..5).forEach { n ->
                            StarButton(
                                n <= current,
                                if (n == maxOf(current, 1)) Modifier.focusRequester(first) else Modifier,
                            ) {
                                rate(n.toUByte())
                            }
                        }
                    }
                    if (current > 0) SettingsRow(Glyph.Close, "Clear rating", "", onClick = { rate(null) })
                }
            }
        }
        if (!touch) LaunchedEffect(page, menu) { runCatching { first.requestFocus() } }
    }
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MenuFrame(subpage: Boolean, onBack: () -> Unit, onDismiss: () -> Unit, content: @Composable () -> Unit) {
    if (!LocalTouch.current) Dialog(onDismissRequest = { if (subpage) onBack() else onDismiss() }, content = content)
    else
        ModalBottomSheet(
            onDismissRequest = onDismiss,
            sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true),
            containerColor = Palette.bg,
        ) {
            BackHandler(subpage, onBack)
            content()
        }
}

@Composable
private fun StarButton(filled: Boolean, modifier: Modifier, onClick: () -> Unit) {
    if (LocalTouch.current)
        return Tap(onClick, modifier.size(52.dp), CircleShape, Palette.surface, Palette.text) {
            GlyphIcon(if (filled) Glyph.Star else Glyph.StarBorder, 28.dp, Palette.text)
        }
    Surface(
        onClick = onClick,
        modifier = modifier.size(52.dp),
        shape = ClickableSurfaceDefaults.shape(CircleShape),
        colors =
            ClickableSurfaceDefaults.colors(
                containerColor = Palette.surface,
                contentColor = Palette.text,
                focusedContainerColor = Palette.tint(0.12f),
                focusedContentColor = Palette.text,
            ),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
        border =
            ClickableSurfaceDefaults.border(
                focusedBorder = Border(BorderStroke(2.dp, Palette.text), shape = CircleShape)
            ),
    ) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            GlyphIcon(if (filled) Glyph.Star else Glyph.StarBorder, 28.dp, Palette.text)
        }
    }
}
