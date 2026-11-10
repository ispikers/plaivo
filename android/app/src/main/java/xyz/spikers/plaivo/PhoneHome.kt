package xyz.spikers.plaivo

import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.interaction.collectIsDraggedAsState
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxWithConstraints
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.pager.HorizontalPager
import androidx.compose.foundation.pager.PagerState
import androidx.compose.foundation.pager.rememberPagerState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Button
import androidx.compose.material3.DropdownMenu
import androidx.compose.material3.DropdownMenuItem
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.material3.pulltorefresh.PullToRefreshDefaults
import androidx.compose.material3.pulltorefresh.rememberPullToRefreshState
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.coerceAtLeast
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.min
import androidx.compose.ui.unit.sp
import coil3.compose.AsyncImage
import kotlinx.coroutines.delay
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.PlexUser
import uniffi.plaivo_ffi.Row as MediaRow
import uniffi.plaivo_ffi.ServerInfo

private const val HERO_ADVANCE_MS = 8000L

@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun PhoneHome(
    server: ServerInfo?,
    serversLoaded: Boolean,
    refresh: Int,
    open: (MediaCard) -> Unit,
    openRow: (String, MediaRow) -> Unit,
    onRefresh: () -> Unit,
    openSearch: () -> Unit,
    openWatchTogether: () -> Unit,
    openDownloads: () -> Unit,
    onConnections: () -> Unit,
    onSignOut: () -> Unit,
) {
    var refreshing by remember { mutableStateOf(false) }
    val home = rememberHomeRows(server, refresh) { refreshing = false }
    val rows = home.rows
    val pull = {
        refreshing = true
        onRefresh()
    }
    val hero = home.hero
    val user by produceState<PlexUser?>(null) { value = io { core.plexUser() } }
    val message =
        when {
            server == null -> if (serversLoaded) "No reachable server" else "Loading…"
            rows.isEmpty() -> home.error ?: "Loading…"
            else -> null
        }
    val failed = if (server == null) serversLoaded else home.error != null
    val state = rememberPullToRefreshState()
    PullToRefreshBox(
        isRefreshing = refreshing,
        onRefresh = pull,
        modifier = Modifier.fillMaxSize(),
        state = state,
        indicator = {
            PullToRefreshDefaults.Indicator(state, refreshing, Modifier.align(Alignment.TopCenter).statusBarsPadding())
        },
    ) {
        BoxWithConstraints(Modifier.fillMaxSize()) {
            val heroHeight = min(maxHeight - 88.dp, maxWidth * 1.5f).coerceAtLeast(280.dp)
            LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 24.dp)) {
                item(key = "top") {
                    Box {
                        if (hero.isNotEmpty()) Hero(hero, open, Modifier.fillMaxWidth().height(heroHeight))
                        TopBar(user, pull, openSearch, openWatchTogether, onConnections, onSignOut)
                    }
                }
                if (message != null)
                    item(key = "message") {
                        Box(Modifier.fillParentMaxWidth().fillParentMaxHeight(0.75f)) {
                            Message(message, openDownloads.takeIf { failed })
                        }
                    }
                touchRows(rows, open) { row -> server?.let { openRow(it.id, row) } }
            }
        }
    }
}

@Composable
private fun TopBar(
    user: PlexUser?,
    onRefresh: () -> Unit,
    openSearch: () -> Unit,
    openWatchTogether: () -> Unit,
    onConnections: () -> Unit,
    onSignOut: () -> Unit,
) {
    Row(
        Modifier.fillMaxWidth().statusBarsPadding().padding(horizontal = 4.dp),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Row(
            Modifier.weight(1f)
                .padding(start = 12.dp, end = 4.dp)
                .height(44.dp)
                .clip(CircleShape)
                .background(Color.Black.copy(alpha = 0.4f))
                .clickable(role = Role.Button, onClick = openSearch)
                .padding(horizontal = 14.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            GlyphIcon(Glyph.Search, 18.dp, Palette.text)
            Text("Search", fontSize = 15.sp, color = Palette.text.copy(alpha = 0.8f))
        }
        IconButton(onClick = onRefresh) { Icon(Glyph.Refresh.vector, "Refresh", tint = Palette.text) }
        IconButton(onClick = openWatchTogether) { Icon(Glyph.Group.vector, "Watch Together", tint = Palette.text) }
        user?.let { Account(it, onConnections, onSignOut) }
    }
}

@Composable
private fun Account(user: PlexUser, onConnections: () -> Unit, onSignOut: () -> Unit) {
    var expanded by remember { mutableStateOf(false) }
    Box {
        IconButton(onClick = { expanded = true }, modifier = Modifier.semantics { contentDescription = "Account" }) {
            Box(
                Modifier.size(32.dp).clip(CircleShape).background(Palette.tint(0.2f)),
                contentAlignment = Alignment.Center,
            ) {
                Text(
                    user.name.take(1).uppercase(),
                    color = Palette.text,
                    fontSize = 14.sp,
                    fontWeight = FontWeight.Bold,
                )
                user.thumb?.let {
                    AsyncImage(
                        it,
                        contentDescription = null,
                        contentScale = ContentScale.Crop,
                        modifier = Modifier.fillMaxSize(),
                    )
                }
            }
        }
        DropdownMenu(expanded = expanded, onDismissRequest = { expanded = false }) {
            Text(
                user.name,
                Modifier.padding(horizontal = 16.dp, vertical = 12.dp),
                color = Palette.muted,
                fontWeight = FontWeight.SemiBold,
            )
            DropdownMenuItem(
                text = { Text("Connections") },
                onClick = {
                    expanded = false
                    onConnections()
                },
                leadingIcon = { Icon(Glyph.Link.vector, null, Modifier.size(20.dp)) },
            )
            DropdownMenuItem(
                text = { Text("Sign out") },
                onClick = {
                    expanded = false
                    onSignOut()
                },
                leadingIcon = { Icon(Glyph.Logout.vector, null, Modifier.size(20.dp)) },
            )
        }
    }
}

@Composable
private fun Hero(cards: List<MediaCard>, open: (MediaCard) -> Unit, modifier: Modifier) {
    val pager = rememberPagerState { cards.size }
    val dragged by pager.interactionSource.collectIsDraggedAsState()
    LaunchedEffect(pager, dragged, cards.size) {
        if (dragged || cards.size < 2) return@LaunchedEffect
        while (true) {
            delay(HERO_ADVANCE_MS)
            pager.animateScrollToPage((pager.currentPage + 1) % cards.size)
        }
    }
    val play = LocalPlay.current
    val bg = Palette.bg
    val scrim =
        remember(bg) {
            Brush.verticalGradient(
                0f to bg.copy(alpha = 0.55f),
                0.2f to Color.Transparent,
                0.45f to Color.Transparent,
                0.95f to bg,
            )
        }
    Box(modifier) {
        HorizontalPager(pager, Modifier.fillMaxSize(), key = { cards[it].key }) { page ->
            val card = cards[page]
            HeroPage(card, scrim, { open(card) }) { play(card) }
        }
        if (cards.size > 1) Dots(cards.size, pager, Modifier.align(Alignment.BottomCenter).padding(bottom = 16.dp))
    }
}

@Composable
private fun HeroPage(card: MediaCard, scrim: Brush, onOpen: () -> Unit, onPlay: () -> Unit) {
    Box(Modifier.fillMaxSize().clickable(interactionSource = null, indication = null, onClick = onOpen)) {
        AsyncImage(
            card.backdrop,
            contentDescription = null,
            contentScale = ContentScale.Crop,
            modifier = Modifier.fillMaxSize(),
        )
        Box(Modifier.fillMaxSize().background(scrim))
        Column(
            Modifier.align(Alignment.BottomCenter)
                .widthIn(max = 560.dp)
                .padding(start = Dims.gutter, end = Dims.gutter, bottom = 40.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
            horizontalAlignment = Alignment.CenterHorizontally,
        ) {
            Text(
                card.title,
                color = Palette.text,
                fontSize = 28.sp,
                lineHeight = 32.sp,
                fontWeight = FontWeight.ExtraBold,
                textAlign = TextAlign.Center,
                maxLines = 2,
                overflow = TextOverflow.Ellipsis,
            )
            if (card.meta.isNotEmpty())
                Text(
                    card.meta,
                    color = Palette.text,
                    fontSize = 13.sp,
                    fontWeight = FontWeight.SemiBold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
            Button(onClick = onPlay, modifier = Modifier.height(48.dp)) {
                Icon(Glyph.Play.vector, null, Modifier.size(22.dp))
                Text(
                    card.left?.let { "Resume · $it" } ?: if (card.progress > 0f) "Resume" else "Play",
                    Modifier.padding(start = 8.dp),
                    fontSize = 15.sp,
                    fontWeight = FontWeight.SemiBold,
                )
                if (card.progress > 0f) {
                    val tint = LocalContentColor.current
                    Box(
                        Modifier.padding(start = 10.dp)
                            .width(40.dp)
                            .height(4.dp)
                            .background(tint.copy(alpha = 0.25f), CircleShape)
                    ) {
                        Box(Modifier.fillMaxWidth(card.progress).fillMaxHeight().background(tint, CircleShape))
                    }
                }
            }
            if (card.summary.isNotEmpty())
                Text(
                    card.summary,
                    color = Palette.text.copy(alpha = 0.78f),
                    fontSize = 14.sp,
                    lineHeight = 20.sp,
                    textAlign = TextAlign.Center,
                    maxLines = 2,
                    overflow = TextOverflow.Ellipsis,
                )
        }
    }
}

@Composable
private fun Dots(count: Int, pager: PagerState, modifier: Modifier) {
    Row(modifier, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
        repeat(count) { i ->
            val active = i == pager.currentPage
            Box(
                Modifier.width(if (active) 24.dp else 8.dp)
                    .height(8.dp)
                    .background(if (active) Palette.text else Palette.tint(0.4f), CircleShape)
            )
        }
    }
}
