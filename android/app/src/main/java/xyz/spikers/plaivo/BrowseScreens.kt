package xyz.spikers.plaivo

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
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
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.tv.material3.Border
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.Surface
import coil3.compose.AsyncImage
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Text
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.Section

private const val PAGE = 120uL
private const val SEARCH_DEBOUNCE_MS = 500L

@Composable
private fun PosterGrid(cards: List<MediaCard>, play: (MediaCard) -> Unit, header: @Composable () -> Unit, onNearEnd: () -> Unit = {}) {
    LazyVerticalGrid(
        columns = GridCells.FixedSize(Dims.poster * LocalCardScale.current + LocalAppearance.current.gap),
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = Dims.edge - 7.dp, end = Dims.edge - 7.dp, top = 28.dp, bottom = 28.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        item(span = { GridItemSpan(maxLineSpan) }) { Box(Modifier.padding(start = 7.dp)) { header() } }
        itemsIndexed(cards, key = { _, c -> "${c.server}/${c.key}" }) { i, card ->
            if (i >= cards.size - 30) LaunchedEffect(cards.size) { onNearEnd() }
            Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.TopCenter) { MediaTile(card, wide = false) { play(card) } }
        }
    }
}

@Composable
fun Heading(text: String) {
    Text(text, fontSize = 24.sp, fontWeight = FontWeight.Bold, color = Palette.text)
}

@Composable
fun LibraryScreen(library: Section, play: (MediaCard) -> Unit) {
    var cards by remember(library) { mutableStateOf<List<MediaCard>>(emptyList()) }
    var total by remember(library) { mutableLongStateOf(-1L) }
    var loading by remember(library) { mutableStateOf(false) }
    var fetched by remember(library) { mutableIntStateOf(0) }
    var error by remember(library) { mutableStateOf<String?>(null) }
    val scope = rememberCoroutineScope()
    fun more() {
        if (loading || (total >= 0 && fetched >= total)) return
        loading = true
        scope.launch {
            io { runCatching { core.libraryPage(library.server, library.key, fetched.toULong(), PAGE) } }
                .onSuccess { page ->
                    cards = (cards + page.cards).distinctBy { it.key }
                    fetched += page.cards.size
                    total = page.total.toLong()
                    error = null
                }
                .onFailure { error = it.message }
            loading = false
        }
    }
    LaunchedEffect(library) { more() }
    val header = @Composable {
        Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
            Heading(library.title)
            val status = error ?: if (total < 0) "Loading…" else "$total items"
            Text(status, fontSize = 13.sp, color = Palette.muted)
        }
    }
    if (LocalAppearance.current.list) PosterList(cards, play, header, ::more) else PosterGrid(cards, play, header, ::more)
}

@Composable
private fun PosterList(cards: List<MediaCard>, play: (MediaCard) -> Unit, header: @Composable () -> Unit, onNearEnd: () -> Unit) {
    val look = LocalAppearance.current
    val menu = LocalCardMenu.current
    val shape = RoundedCornerShape(12.dp)
    val scale = (look.listPoster.value - 70f) / 50f
    LazyColumn(
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(horizontal = Dims.edge, vertical = 28.dp),
        verticalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        item { Box(Modifier.padding(bottom = 10.dp)) { header() } }
        itemsIndexed(cards, key = { _, c -> "${c.server}/${c.key}" }) { i, card ->
            if (i >= cards.size - 30) LaunchedEffect(cards.size) { onNearEnd() }
            Surface(
                onClick = { play(card) },
                onLongClick = menu?.let { { it(card) } },
                modifier = Modifier.fillMaxWidth(),
                shape = ClickableSurfaceDefaults.shape(shape),
                colors = ClickableSurfaceDefaults.colors(containerColor = Palette.bg, contentColor = Palette.text, focusedContainerColor = Palette.tint(0.08f), focusedContentColor = Palette.text),
                scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
                border = ClickableSurfaceDefaults.border(focusedBorder = Border(BorderStroke(2.dp, Palette.text), shape = shape)),
            ) {
                Row(Modifier.padding(8.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                    Box(Modifier.width(look.listPoster).aspectRatio(2f / 3f).clip(RoundedCornerShape(8.dp)).background(Palette.surface)) {
                        AsyncImage(model = card.image, contentDescription = card.title, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
                        if (card.watched && look.watchedIndicators) {
                            Box(Modifier.align(Alignment.TopEnd).padding(4.dp).size(20.dp).background(Palette.text, CircleShape), contentAlignment = Alignment.Center) { GlyphIcon(Glyph.Check, 14.dp, Palette.bg) }
                        }
                    }
                    Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(card.title, fontSize = (13 + 3 * scale).sp, fontWeight = FontWeight.SemiBold, color = Palette.text, maxLines = 2, overflow = TextOverflow.Ellipsis)
                        Text(card.meta, fontSize = (10 + 3 * scale).sp, color = Palette.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        Text(card.summary, fontSize = (10 + 3 * scale).sp, lineHeight = ((10 + 3 * scale) * 1.3f).sp, color = Palette.tint(0.5f), maxLines = look.summaryLines, overflow = TextOverflow.Ellipsis)
                    }
                }
            }
        }
    }
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
    PosterGrid(results, play, header = {
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
                        Modifier
                            .height(44.dp)
                            .background(Palette.surface, CircleShape)
                            .border(BorderStroke(if (focused) 2.dp else 1.dp, if (focused) Palette.text else Palette.outline), CircleShape)
                            .padding(horizontal = 16.dp),
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        GlyphIcon(Glyph.Search, 20.dp, Palette.muted)
                        Box(Modifier.weight(1f)) {
                            if (query.isEmpty()) Text("Search your libraries", fontSize = 16.sp, color = Palette.muted)
                            field()
                        }
                    }
                },
            )
            val status = when {
                searching -> "Searching…"
                query.isNotBlank() && results.isEmpty() -> "No results"
                else -> ""
            }
            if (status.isNotEmpty()) Text(status, fontSize = 13.sp, color = Palette.muted)
        }
    })
}
