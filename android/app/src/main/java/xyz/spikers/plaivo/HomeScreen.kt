package xyz.spikers.plaivo

import androidx.compose.animation.Crossfade
import androidx.compose.animation.core.tween
import androidx.compose.foundation.ExperimentalFoundationApi
import androidx.compose.foundation.background
import androidx.compose.foundation.gestures.BringIntoViewSpec
import androidx.compose.foundation.gestures.LocalBringIntoViewSpec
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.focusRestorer
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.Shadow
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.em
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.coroutineScope
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.DownloadState
import uniffi.plaivo_ffi.Home
import uniffi.plaivo_ffi.Row as MediaRow
import uniffi.plaivo_ffi.ServerInfo

private const val SPOTLIGHT_DEBOUNCE_MS = 150L
private const val CROSSFADE_MS = 280

@OptIn(ExperimentalFoundationApi::class)
private val keepScroll =
    object : BringIntoViewSpec {
        override fun calculateScrollDistance(offset: Float, size: Float, containerSize: Float) = 0f
    }

@OptIn(ExperimentalFoundationApi::class)
private val centerFocused =
    object : BringIntoViewSpec {
        override fun calculateScrollDistance(offset: Float, size: Float, containerSize: Float) =
            offset - (containerSize - size) / 2f
    }

@Composable
fun HomeScreen(
    server: ServerInfo?,
    serversLoaded: Boolean,
    refresh: Int,
    takeFocus: Boolean,
    onFocusTaken: () -> Unit,
    play: (MediaCard) -> Unit,
    openRow: (MediaRow) -> Unit,
    openDownloads: () -> Unit,
) {
    val home = rememberHomeRows(server, serversLoaded, refresh)
    when {
        server == null ->
            Message(if (serversLoaded) "No reachable server" else "Loading…", openDownloads.takeIf { serversLoaded })
        home.rows.isEmpty() -> Message(home.error ?: "Loading…", openDownloads.takeIf { home.error != null })
        else -> Spotlight(home.rows, takeFocus, onFocusTaken, play, openRow)
    }
}

class HomeRows {
    var rows by mutableStateOf<List<MediaRow>>(emptyList())
        private set

    var hero by mutableStateOf<List<MediaCard>>(emptyList())
        private set

    var error by mutableStateOf<String?>(null)
    private var base: List<MediaRow>? = null
    private var resume: MediaRow? = null

    fun show(home: Home) {
        base = home.rows
        hero = home.hero
        publish()
    }

    fun showResume(row: MediaRow?) {
        resume = row
        publish()
    }

    private fun publish() {
        rows = base?.let { listOfNotNull(resume) + it } ?: emptyList()
    }
}

@Composable
fun rememberHomeRows(server: ServerInfo?, serversLoaded: Boolean, refresh: Int, onLoaded: () -> Unit = {}): HomeRows {
    val home = remember(server?.id) { HomeRows() }
    var retriedAfterDiscovery by remember(server?.id, refresh) { mutableStateOf(false) }
    LaunchedEffect(serversLoaded, home.error) { if (serversLoaded && home.error != null) retriedAfterDiscovery = true }
    LaunchedEffect(server?.id, refresh, retriedAfterDiscovery) {
        if (server != null) {
            val id = server.id
            val others = io { core.servers() }.map { it.id }.filter { it != id }
            if (home.rows.isEmpty()) {
                val cached = coroutineScope {
                    others.forEach { launch { io { core.refreshContinue(it, true) } } }
                    io { runCatching { core.home(id, true) } }
                }
                cached.onSuccess {
                    if (it.rows.isNotEmpty()) {
                        home.showResume(core.continueRow(id))
                        home.show(it)
                    }
                }
            }
            others.forEach {
                launch {
                    io { core.refreshContinue(it, false) }
                    home.showResume(core.continueRow(id))
                }
            }
            io { runCatching { core.home(id, false) } }
                .onSuccess {
                    home.showResume(core.continueRow(id))
                    home.show(it)
                    home.error = null
                }
                .onFailure {
                    val offline = core.serverOffline(id)
                    if (offline) {
                        home.showResume(core.continueRow(id))
                        home.show(Home(emptyList(), emptyList()))
                    }
                    if (home.rows.isEmpty())
                        home.error = if (offline) "Can't reach ${server.name} right now." else it.message
                }
        }
        onLoaded()
    }
    return home
}

@Composable
fun Message(text: String, openDownloads: (() -> Unit)? = null) {
    val entries by DownloadQueue.entries.collectAsState()
    val offline = openDownloads?.takeIf { entries.any { it.state == DownloadState.DONE } }
    Column(
        Modifier.fillMaxSize(),
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text(text, color = Palette.muted)
        if (offline != null) ActionButton("Watch your downloads", offline)
    }
}

@Composable
fun Spotlight(
    rows: List<MediaRow>,
    takeFocus: Boolean,
    onFocusTaken: () -> Unit,
    play: (MediaCard) -> Unit,
    openRow: (MediaRow) -> Unit,
) {
    var focused by remember { mutableStateOf(rows.first().cards.firstOrNull()) }
    var shown by remember { mutableStateOf(focused) }
    LaunchedEffect(focused) {
        delay(SPOTLIGHT_DEBOUNCE_MS)
        shown = focused
    }
    Box(Modifier.fillMaxSize()) {
        Crossfade(shown?.backdrop, animationSpec = tween(CROSSFADE_MS), label = "backdrop") { url ->
            if (url != null)
                AsyncImage(
                    model = url,
                    contentDescription = null,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize(),
                )
        }
        Box(
            Modifier.fillMaxSize()
                .background(
                    Brush.horizontalGradient(
                        0f to Palette.bg.copy(alpha = 0.86f),
                        0.56f to Palette.bg.copy(alpha = 0.32f),
                        1f to Color.Transparent,
                    )
                )
        )
        Box(
            Modifier.fillMaxSize()
                .background(
                    Brush.verticalGradient(
                        0f to Color.Black.copy(alpha = 0.45f),
                        0.38f to Color.Transparent,
                        1f to Palette.bg.copy(alpha = 0.96f),
                    )
                )
        )
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val railHeight = maxHeight * 0.52f
            val top = maxHeight * 0.075f
            Info(
                shown,
                Modifier.align(Alignment.TopStart)
                    .padding(start = Dims.edge, top = top)
                    .width(maxWidth * 0.57f)
                    .height(maxHeight - railHeight - top - 8.dp),
            )
            Rails(
                rows,
                takeFocus,
                onFocusTaken,
                { focused = it },
                play,
                openRow,
                Modifier.align(Alignment.BottomStart).fillMaxWidth().height(railHeight),
            )
        }
    }
}

@Composable
private fun Info(card: MediaCard?, modifier: Modifier) {
    Crossfade(card, animationSpec = tween(CROSSFADE_MS), modifier = modifier, label = "info") { c ->
        if (c == null) return@Crossfade
        Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(8.dp, Alignment.Bottom)) {
            Text(
                c.title,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
                style =
                    TextStyle(
                        fontSize = 30.sp,
                        fontWeight = FontWeight.Bold,
                        color = Palette.text,
                        shadow = Shadow(Color.Black.copy(alpha = 0.5f), blurRadius = 12f),
                    ),
            )
            val meta =
                listOf(c.subtitle.takeIf { c.kind == "episode" }, c.meta)
                    .filterNot { it.isNullOrEmpty() }
                    .joinToString(" · ")
            if (meta.isNotEmpty())
                Text(
                    meta,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.Bold,
                    color = Palette.text,
                )
            if (c.summary.isNotEmpty()) {
                Text(
                    c.summary,
                    maxLines = 3,
                    overflow = TextOverflow.Ellipsis,
                    fontSize = 15.sp,
                    lineHeight = 1.34.em,
                    color = Palette.text.copy(alpha = 0.78f),
                )
            }
        }
    }
}

@OptIn(ExperimentalFoundationApi::class, ExperimentalComposeUiApi::class)
@Composable
private fun Rails(
    rows: List<MediaRow>,
    takeFocus: Boolean,
    onFocusTaken: () -> Unit,
    onFocus: (MediaCard) -> Unit,
    play: (MediaCard) -> Unit,
    openRow: (MediaRow) -> Unit,
    modifier: Modifier,
) {
    val list = rememberLazyListState()
    val scope = rememberCoroutineScope()
    val playNow = LocalPlay.current
    var active by remember { mutableStateOf(rows.firstOrNull()?.title) }
    val first = remember { FocusRequester() }
    LaunchedEffect(takeFocus) {
        if (takeFocus) {
            runCatching { first.requestFocus() }
            onFocusTaken()
        }
    }
    CompositionLocalProvider(LocalBringIntoViewSpec provides keepScroll) {
        LazyColumn(
            state = list,
            modifier = modifier,
            contentPadding = PaddingValues(top = 12.dp, bottom = 400.dp),
            userScrollEnabled = false,
        ) {
            itemsIndexed(rows, key = { _, row -> row.title }) { index, row ->
                Column(
                    Modifier.onFocusChanged {
                        if (it.hasFocus && active != row.title) {
                            active = row.title
                            scope.launch { list.animateScrollToItem(index) }
                        }
                    }
                ) {
                    val isActive = row.title == active
                    Row(
                        Modifier.padding(start = Dims.edge).height(30.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                    ) {
                        GlyphIcon(
                            if (row.resume) Glyph.PlayCircle else Glyph.Sparkle,
                            18.dp,
                            Palette.text.copy(alpha = if (isActive) 1f else 0.54f),
                        )
                        Text(
                            row.title,
                            fontSize = 17.sp,
                            fontWeight = if (isActive) FontWeight.ExtraBold else FontWeight.Bold,
                            color = Palette.text.copy(alpha = if (isActive) 1f else 0.54f),
                        )
                    }
                    CompositionLocalProvider(LocalBringIntoViewSpec provides centerFocused) {
                        LazyRow(
                            modifier = Modifier.fillMaxWidth().focusRestorer(),
                            contentPadding = PaddingValues(horizontal = Dims.edge, vertical = 6.dp),
                            horizontalArrangement = Arrangement.spacedBy(LocalAppearance.current.gap),
                        ) {
                            itemsIndexed(row.cards, key = { _, c -> "${c.server}/${c.key}" }) { i, card ->
                                MediaTile(
                                    card,
                                    row.wide,
                                    modifier = if (index == 0 && i == 0) Modifier.focusRequester(first) else Modifier,
                                    dim = !isActive,
                                    onFocus = { onFocus(card) },
                                ) {
                                    if (row.resume) playNow(card) else play(card)
                                }
                            }
                            if (row.key.isNotEmpty())
                                item(key = "see-all") { SeeAll(row.wide, dim = !isActive) { openRow(row) } }
                        }
                    }
                }
            }
        }
    }
}
