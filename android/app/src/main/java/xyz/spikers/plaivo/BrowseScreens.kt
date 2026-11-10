package xyz.spikers.plaivo

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.combinedClickable
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.FilterChip
import androidx.compose.material3.LocalContentColor as M3ContentColor
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text as M3Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalSoftwareKeyboardController
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.compose.ui.window.DialogProperties
import androidx.tv.material3.Border
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.HubList
import uniffi.plaivo_ffi.Listing
import uniffi.plaivo_ffi.Row as MediaRow
import uniffi.plaivo_ffi.Section
import uniffi.plaivo_ffi.gridColumns
import uniffi.plaivo_ffi.librarySorts

private const val PAGE = 120uL
private const val SEARCH_DEBOUNCE_MS = 500L
private val TOUCH_POSTER = 150.dp

@Composable
private fun PosterGrid(
    cards: List<MediaCard>,
    play: (MediaCard) -> Unit,
    header: @Composable () -> Unit,
    onNearEnd: () -> Unit = {},
    focusFirst: Boolean = false,
    refreshing: Boolean = false,
    onRefresh: (() -> Unit)? = null,
) {
    if (LocalTouch.current) return TouchPosterGrid(cards, play, header, onNearEnd, refreshing, onRefresh)
    val first = remember { FocusRequester() }
    if (focusFirst) LaunchedEffect(cards.isNotEmpty()) { if (cards.isNotEmpty()) runCatching { first.requestFocus() } }
    LazyVerticalGrid(
        columns = GridCells.FixedSize(Dims.poster * LocalCardScale.current + LocalAppearance.current.gap),
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = Dims.edge - 7.dp, end = Dims.edge - 7.dp, top = 28.dp, bottom = 28.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        item(span = { GridItemSpan(maxLineSpan) }) { Box(Modifier.padding(start = 7.dp)) { header() } }
        itemsIndexed(cards, key = { _, c -> "${c.server}/${c.key}" }) { i, card ->
            if (i >= cards.size - 30) LaunchedEffect(cards.size) { onNearEnd() }
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
                MediaTile(card, wide = false, if (i == 0) Modifier.focusRequester(first) else Modifier) { play(card) }
            }
        }
    }
}

@Composable
internal fun touchColumns(width: Dp): Int {
    val card = TOUCH_POSTER * LocalCardScale.current
    val gap = LocalAppearance.current.gap
    return remember(width, card, gap) { gridColumns((width - Dims.gutter * 2).value, card.value, gap.value) }
}

@Composable
private fun TouchPosterGrid(
    cards: List<MediaCard>,
    play: (MediaCard) -> Unit,
    header: @Composable () -> Unit,
    onNearEnd: () -> Unit,
    refreshing: Boolean,
    onRefresh: (() -> Unit)?,
) {
    BoxWithConstraints(Modifier.fillMaxSize()) {
        val columns = touchColumns(maxWidth)
        Refreshable(refreshing, onRefresh) {
            LazyVerticalGrid(
                columns = GridCells.Fixed(columns),
                modifier = Modifier.fillMaxSize(),
                contentPadding = PaddingValues(Dims.gutter),
                horizontalArrangement = Arrangement.spacedBy(LocalAppearance.current.gap),
                verticalArrangement = Arrangement.spacedBy(18.dp),
            ) {
                item(span = { GridItemSpan(maxLineSpan) }) { header() }
                itemsIndexed(cards, key = { _, c -> "${c.server}/${c.key}" }) { i, card ->
                    if (i >= cards.size - 30) LaunchedEffect(cards.size) { onNearEnd() }
                    MediaTile(card, wide = false, Modifier.fillMaxWidth()) { play(card) }
                }
            }
        }
    }
}

@Composable
private fun Refreshable(refreshing: Boolean, onRefresh: (() -> Unit)?, content: @Composable () -> Unit) {
    if (onRefresh == null) content()
    else
        PullToRefreshBox(isRefreshing = refreshing, onRefresh = onRefresh, modifier = Modifier.fillMaxSize()) {
            content()
        }
}

@Composable
fun Heading(text: String) {
    Text(text, fontSize = 24.sp, fontWeight = FontWeight.Bold, color = Palette.text)
}

enum class LibTab(val label: String) {
    Recommended("Recommended"),
    Browse("Browse"),
    Collections("Collections"),
}

@Composable
fun LibraryScreen(
    library: Section,
    tab: LibTab,
    onTab: (LibTab) -> Unit,
    open: (MediaCard) -> Unit,
    openRow: (MediaRow) -> Unit,
) {
    val sorts = remember { librarySorts() }
    var sort by remember { mutableStateOf(sorts.first().id) }
    Column(Modifier.fillMaxSize()) {
        TabHeader(library.title, LibTab.entries.map { it.label }, tab.ordinal) { onTab(LibTab.entries[it]) }
        Box(Modifier.weight(1f)) {
            when (tab) {
                LibTab.Recommended -> Recommended(library, open, openRow)
                LibTab.Browse ->
                    PagedGrid(library.server, library.key, Listing.Items(sort), open) { status ->
                        SortRow(status, sorts.map { it.label }, sorts.indexOfFirst { it.id == sort }) {
                            sort = sorts[it].id
                        }
                    }
                LibTab.Collections -> PagedGrid(library.server, library.key, Listing.Collections, open) { Status(it) }
            }
        }
    }
}

@Composable
fun TabHeader(title: String, labels: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    if (LocalTouch.current)
        Column(Modifier.padding(top = 12.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Box(Modifier.padding(horizontal = Dims.gutter)) { Heading(title) }
            FilterChips(labels, selected, onSelect, Dims.gutter)
        }
    else
        Row(
            Modifier.padding(start = Dims.edge, end = Dims.edge, top = 24.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(20.dp),
        ) {
            Heading(title)
            Chips(labels, selected, onSelect)
        }
}

@Composable
fun SortRow(status: String, labels: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically) {
        Status(status, Modifier.weight(1f))
        if (LocalTouch.current) SortMenu(labels, selected, onSelect) else Chips(labels, selected, onSelect)
    }
}

@Composable
private fun SortMenu(labels: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    if (labels.isEmpty()) return
    var open by remember { mutableStateOf(false) }
    Box {
        FilterChip(
            selected = false,
            onClick = { open = true },
            label = { M3Text(labels.getOrNull(selected) ?: "Sort") },
            leadingIcon = { GlyphIcon(Glyph.Sort, 18.dp, M3ContentColor.current) },
        )
        DropdownMenu(expanded = open, onDismissRequest = { open = false }) {
            labels.forEachIndexed { i, label ->
                DropdownMenuItem(
                    text = { M3Text(label) },
                    onClick = {
                        open = false
                        onSelect(i)
                    },
                    trailingIcon =
                        if (i == selected) {
                            { GlyphIcon(Glyph.Check, 18.dp, M3ContentColor.current) }
                        } else null,
                )
            }
        }
    }
}

@Composable
private fun Recommended(library: Section, open: (MediaCard) -> Unit, openRow: (MediaRow) -> Unit) {
    var rows by remember { mutableStateOf<List<MediaRow>?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var loads by remember { mutableIntStateOf(0) }
    var refreshing by remember { mutableStateOf(false) }
    LaunchedEffect(loads) {
        if (loads == 0)
            io { runCatching { core.libraryHubs(library.server, library.key, library.movies, true) } }
                .onSuccess { if (it.isNotEmpty()) rows = it }
        io { runCatching { core.libraryHubs(library.server, library.key, library.movies, false) } }
            .onSuccess {
                rows = it
                error = null
            }
            .onFailure { if (rows == null) error = it.message }
        refreshing = false
    }
    val r = rows
    val message = error ?: if (r == null) "Loading…" else "Nothing to recommend yet"
    if (LocalTouch.current)
        Refreshable(
            refreshing,
            {
                refreshing = true
                loads++
            },
        ) {
            if (r.isNullOrEmpty())
                LazyColumn(Modifier.fillMaxSize()) { item { Box(Modifier.fillParentMaxSize()) { Message(message) } } }
            else TouchRows(r, open, openRow)
        }
    else if (r.isNullOrEmpty()) Message(message) else Spotlight(r, false, {}, open, openRow)
}

@Composable
private fun TouchRows(rows: List<MediaRow>, open: (MediaCard) -> Unit, openRow: (MediaRow) -> Unit) {
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(top = 12.dp)) { touchRows(rows, open, openRow) }
}

fun LazyListScope.touchRows(rows: List<MediaRow>, open: (MediaCard) -> Unit, openRow: (MediaRow) -> Unit) {
    items(rows, key = { it.title }) { row -> TouchRow(row, open, openRow) }
}

@Composable
private fun TouchRow(row: MediaRow, open: (MediaCard) -> Unit, openRow: (MediaRow) -> Unit) {
    val playNow = LocalPlay.current
    Column(Modifier.padding(bottom = 16.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        val more = row.key.isNotEmpty()
        Row(
            Modifier.fillMaxWidth()
                .then(if (more) Modifier.clickable { openRow(row) } else Modifier)
                .padding(horizontal = Dims.gutter, vertical = 6.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            GlyphIcon(if (row.resume) Glyph.PlayCircle else Glyph.Sparkle, 18.dp, Palette.text)
            Text(
                row.title,
                fontSize = 17.sp,
                fontWeight = FontWeight.Bold,
                color = Palette.text,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
                modifier = Modifier.weight(1f, fill = false),
            )
            if (more) GlyphIcon(Glyph.ChevronRight, 20.dp, Palette.text)
        }
        LazyRow(
            contentPadding = PaddingValues(horizontal = Dims.gutter),
            horizontalArrangement = Arrangement.spacedBy(LocalAppearance.current.gap),
        ) {
            items(row.cards, key = { "${it.server}/${it.key}" }) { card ->
                MediaTile(card, row.wide) { if (row.resume) playNow(card) else open(card) }
            }
        }
    }
}

@Composable
fun HubListScreen(server: String, key: String, title: String, open: (MediaCard) -> Unit) {
    var list by remember { mutableStateOf<HubList?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var sort by remember { mutableStateOf("") }
    var loads by remember { mutableIntStateOf(0) }
    var refreshing by remember { mutableStateOf(false) }
    LaunchedEffect(loads) {
        io { runCatching { core.hubList(server, key) } }
            .onSuccess {
                list = it
                if (loads == 0) sort = it.initial
                error = null
            }
            .onFailure { error = it.message }
        refreshing = false
    }
    val l = list
    val cards =
        remember(l, sort) {
            val order = l?.sorts?.firstOrNull { it.id == sort }?.order
            (order?.map { l.cards[it.toInt()] } ?: l?.cards.orEmpty()).distinctBy { "${it.server}/${it.key}" }
        }
    val status =
        error
            ?: when {
                l == null -> "Loading…"
                cards.isEmpty() -> "Nothing here"
                else -> "${cards.size} items"
            }
    val labels = remember(l) { l?.sorts?.map { it.label }.orEmpty() }
    PosterGrid(
        cards,
        open,
        focusFirst = true,
        refreshing = refreshing,
        onRefresh = {
            refreshing = true
            loads++
        },
        header = {
            Column(verticalArrangement = Arrangement.spacedBy(10.dp)) {
                Heading(title)
                SortRow(status, labels, l?.sorts?.indexOfFirst { it.id == sort } ?: -1) { i ->
                    l?.let { sort = it.sorts[i].id }
                }
            }
        },
    )
}

@Composable
fun CollectionScreen(server: String, key: String, title: String, open: (MediaCard) -> Unit) {
    PagedGrid(server, key, Listing.CollectionItems, open, focusFirst = true) { status ->
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Heading(title)
            Status(status)
        }
    }
}

@Composable
fun PagedGrid(
    server: String,
    key: String,
    listing: Listing,
    open: (MediaCard) -> Unit,
    focusFirst: Boolean = false,
    header: @Composable (String) -> Unit,
) {
    var cards by remember(listing) { mutableStateOf<List<MediaCard>>(emptyList()) }
    var total by remember(listing) { mutableLongStateOf(-1L) }
    var loading by remember(listing) { mutableStateOf(false) }
    var refreshing by remember(listing) { mutableStateOf(false) }
    var fetched by remember(listing) { mutableIntStateOf(0) }
    var error by remember(listing) { mutableStateOf<String?>(null) }
    var job by remember(listing) { mutableStateOf<Job?>(null) }
    val scope = rememberCoroutineScope()
    fun load(reset: Boolean) {
        if (!reset && (loading || (total >= 0 && fetched >= total))) return
        job?.cancel()
        loading = true
        refreshing = reset
        job =
            scope.launch {
                val start = if (reset) 0 else fetched
                io { runCatching { core.libraryPage(server, key, listing, start.toULong(), PAGE) } }
                    .onSuccess { page ->
                        cards = ((if (reset) emptyList() else cards) + page.cards).distinctBy { it.key }
                        fetched = start + page.cards.size
                        total = page.total.toLong()
                        error = null
                    }
                    .onFailure { error = it.message }
                loading = false
                refreshing = false
            }
    }
    fun more() = load(false)
    fun reload() = load(true)
    LaunchedEffect(listing) { more() }
    val status =
        error
            ?: when {
                total < 0 -> "Loading…"
                total == 0L && listing == Listing.Collections -> "No collections"
                total == 0L && listing is Listing.Watchlist -> "Nothing on your watchlist"
                total == 0L -> "Nothing here"
                else -> "$total items"
            }
    if (LocalAppearance.current.list)
        PosterList(cards, open, { header(status) }, ::more, focusFirst, refreshing, ::reload)
    else PosterGrid(cards, open, { header(status) }, ::more, focusFirst, refreshing, ::reload)
}

@Composable
private fun Status(text: String, modifier: Modifier = Modifier) {
    Text(text, fontSize = 13.sp, color = Palette.muted, modifier = modifier)
}

@Composable
fun Chips(labels: List<String>, selected: Int, onSelect: (Int) -> Unit) {
    if (LocalTouch.current) return FilterChips(labels, selected, onSelect)
    Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        labels.forEachIndexed { i, label ->
            val on = i == selected
            Surface(
                onClick = { onSelect(i) },
                modifier = Modifier.height(36.dp),
                shape = ClickableSurfaceDefaults.shape(CircleShape),
                colors =
                    ClickableSurfaceDefaults.colors(
                        containerColor = if (on) Palette.text else Palette.surface,
                        contentColor = if (on) Palette.bg else Palette.text,
                        focusedContainerColor = if (on) Palette.text else Palette.tint(0.12f),
                        focusedContentColor = if (on) Palette.bg else Palette.text,
                    ),
                scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
                border =
                    ClickableSurfaceDefaults.border(
                        border =
                            Border(BorderStroke(1.dp, if (on) Palette.text else Palette.outline), shape = CircleShape),
                        focusedBorder = Border(BorderStroke(2.dp, Palette.text), inset = (-3).dp, shape = CircleShape),
                    ),
            ) {
                Box(Modifier.fillMaxHeight().padding(horizontal = 16.dp), contentAlignment = Alignment.Center) {
                    Text(label, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
                }
            }
        }
    }
}

@Composable
private fun FilterChips(labels: List<String>, selected: Int, onSelect: (Int) -> Unit, inset: Dp = 0.dp) {
    Row(
        Modifier.horizontalScroll(rememberScrollState()).padding(horizontal = inset),
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        labels.forEachIndexed { i, label ->
            FilterChip(selected = i == selected, onClick = { onSelect(i) }, label = { M3Text(label) })
        }
    }
}

@Composable
fun FullPage(onBack: () -> Unit, content: @Composable () -> Unit) {
    Dialog(
        onDismissRequest = onBack,
        properties = DialogProperties(usePlatformDefaultWidth = false, decorFitsSystemWindows = false),
    ) {
        Box(Modifier.fillMaxSize().background(Palette.bg)) { content() }
    }
}

@Composable
private fun PosterList(
    cards: List<MediaCard>,
    play: (MediaCard) -> Unit,
    header: @Composable () -> Unit,
    onNearEnd: () -> Unit,
    focusFirst: Boolean,
    refreshing: Boolean,
    onRefresh: () -> Unit,
) {
    val touch = LocalTouch.current
    val look = LocalAppearance.current
    val first = remember { FocusRequester() }
    if (focusFirst && !touch)
        LaunchedEffect(cards.isNotEmpty()) { if (cards.isNotEmpty()) runCatching { first.requestFocus() } }
    val menu = LocalCardMenu.current
    val shape = RoundedCornerShape(12.dp)
    val scale = (look.listPoster.value - 70f) / 50f
    Refreshable(refreshing, if (touch) onRefresh else null) {
        LazyColumn(
            modifier = Modifier.fillMaxSize(),
            contentPadding =
                if (touch) PaddingValues(Dims.gutter) else PaddingValues(horizontal = Dims.edge, vertical = 28.dp),
            verticalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            item { Box(Modifier.padding(bottom = 10.dp)) { header() } }
            itemsIndexed(cards, key = { _, c -> "${c.server}/${c.key}" }) { i, card ->
                if (i >= cards.size - 30) LaunchedEffect(cards.size) { onNearEnd() }
                if (touch)
                    Box(
                        Modifier.fillMaxWidth().clip(shape).combinedClickable(
                            onLongClick = menu?.let { { it(card) } }
                        ) {
                            play(card)
                        }
                    ) {
                        ListRow(card, look, scale)
                    }
                else
                    Surface(
                        onClick = { play(card) },
                        onLongClick = menu?.let { { it(card) } },
                        modifier =
                            if (i == 0) Modifier.fillMaxWidth().focusRequester(first) else Modifier.fillMaxWidth(),
                        shape = ClickableSurfaceDefaults.shape(shape),
                        colors =
                            ClickableSurfaceDefaults.colors(
                                containerColor = Palette.bg,
                                contentColor = Palette.text,
                                focusedContainerColor = Palette.tint(0.08f),
                                focusedContentColor = Palette.text,
                            ),
                        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
                        border =
                            ClickableSurfaceDefaults.border(
                                focusedBorder = Border(BorderStroke(2.dp, Palette.text), shape = shape)
                            ),
                    ) {
                        ListRow(card, look, scale)
                    }
            }
        }
    }
}

@Composable
private fun ListRow(card: MediaCard, look: Appearance, scale: Float) {
    Row(Modifier.padding(8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
        Box(
            Modifier.width(look.listPoster)
                .aspectRatio(2f / 3f)
                .clip(RoundedCornerShape(8.dp))
                .background(Palette.surface)
        ) {
            AsyncImage(
                model = card.image,
                contentDescription = card.title,
                contentScale = ContentScale.Crop,
                modifier = Modifier.fillMaxSize(),
            )
            if (card.watched && look.watchedIndicators) {
                Box(
                    Modifier.align(Alignment.TopEnd).padding(4.dp).size(20.dp).background(Palette.text, CircleShape),
                    contentAlignment = Alignment.Center,
                ) {
                    GlyphIcon(Glyph.Check, 14.dp, Palette.bg)
                }
            }
        }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Text(
                card.title,
                fontSize = (13 + 3 * scale).sp,
                fontWeight = FontWeight.SemiBold,
                color = Palette.text,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                card.meta,
                fontSize = (10 + 3 * scale).sp,
                color = Palette.muted,
                maxLines = 1,
                overflow = TextOverflow.Ellipsis,
            )
            Text(
                card.summary,
                fontSize = (10 + 3 * scale).sp,
                lineHeight = ((10 + 3 * scale) * 1.3f).sp,
                color = Palette.tint(0.5f),
                maxLines = look.summaryLines,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private fun searchStatus(searching: Boolean, query: String, results: List<MediaCard>) =
    when {
        searching -> "Searching…"
        query.isNotBlank() && results.isEmpty() -> "No results"
        else -> ""
    }

@Composable
fun SearchScreen(play: (MediaCard) -> Unit) {
    var query by rememberSaveable { mutableStateOf("") }
    var results by remember { mutableStateOf<List<MediaCard>>(emptyList()) }
    var searching by remember { mutableStateOf(false) }
    var focused by remember { mutableStateOf(false) }
    LaunchedEffect(query) {
        val q = query.trim()
        if (q.isEmpty()) {
            results = emptyList()
            return@LaunchedEffect
        }
        delay(SEARCH_DEBOUNCE_MS)
        searching = true
        results = io { core.search(q) }
        searching = false
    }
    if (LocalTouch.current)
        return TouchSearch(query, { query = it }, results, searchStatus(searching, query, results), play)
    PosterGrid(
        results,
        play,
        header = {
            Column(verticalArrangement = Arrangement.spacedBy(12.dp)) {
                BasicTextField(
                    value = query,
                    onValueChange = { query = it },
                    singleLine = true,
                    textStyle = TextStyle(fontSize = 16.sp, color = Palette.text),
                    cursorBrush = SolidColor(Palette.text),
                    keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
                    keyboardActions = KeyboardActions(onSearch = {}),
                    modifier = Modifier.widthIn(max = 520.dp).fillMaxWidth().onFocusChanged { focused = it.isFocused },
                    decorationBox = { field ->
                        Row(
                            Modifier.height(44.dp)
                                .background(Palette.surface, CircleShape)
                                .border(
                                    BorderStroke(
                                        if (focused) 2.dp else 1.dp,
                                        if (focused) Palette.text else Palette.outline,
                                    ),
                                    CircleShape,
                                )
                                .padding(horizontal = 16.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            GlyphIcon(Glyph.Search, 20.dp, Palette.muted)
                            Box(Modifier.weight(1f)) {
                                if (query.isEmpty())
                                    Text("Search your libraries", fontSize = 16.sp, color = Palette.muted)
                                field()
                            }
                        }
                    },
                )
                val status = searchStatus(searching, query, results)
                if (status.isNotEmpty()) Text(status, fontSize = 13.sp, color = Palette.muted)
            }
        },
    )
}

@Composable
private fun TouchSearch(
    query: String,
    onQuery: (String) -> Unit,
    results: List<MediaCard>,
    status: String,
    play: (MediaCard) -> Unit,
) {
    val focus = remember { FocusRequester() }
    val keyboard = LocalSoftwareKeyboardController.current
    LaunchedEffect(Unit) {
        runCatching { focus.requestFocus() }
        keyboard?.show()
    }
    Column(Modifier.fillMaxSize()) {
        OutlinedTextField(
            value = query,
            onValueChange = onQuery,
            modifier =
                Modifier.fillMaxWidth()
                    .padding(start = Dims.gutter, end = Dims.gutter, top = 12.dp)
                    .focusRequester(focus),
            placeholder = { M3Text("Search your libraries") },
            leadingIcon = { GlyphIcon(Glyph.Search, 20.dp, Palette.muted) },
            singleLine = true,
            shape = CircleShape,
            keyboardOptions = KeyboardOptions(imeAction = ImeAction.Search),
            keyboardActions = KeyboardActions(onSearch = { keyboard?.hide() }),
        )
        Box(Modifier.weight(1f)) { PosterGrid(results, play, header = { if (status.isNotEmpty()) Status(status) }) }
    }
}
