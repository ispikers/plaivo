package xyz.spikers.plaivo

import android.widget.Toast
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.horizontalScroll
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.BoxScope
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.sizeIn
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.LazyListScope
import androidx.compose.foundation.lazy.LazyRow
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalWindowInfo
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Border
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.LocalContentColor
import androidx.tv.material3.LocalTextStyle
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.launch
import uniffi.plaivo_ffi.Action
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.CastMember
import uniffi.plaivo_ffi.Detail
import uniffi.plaivo_ffi.RatingSource
import uniffi.plaivo_ffi.Row as MediaRow
import uniffi.plaivo_ffi.discoverId

private val rowShape = RoundedCornerShape(10.dp)
private val TV_WIDTH = 960.dp

private fun Detail.card(key: String = this.key) =
    MediaCard(
        server = server,
        key = key,
        kind = kind,
        title = title,
        subtitle = "",
        image = null,
        backdrop = backdrop,
        meta = "",
        summary = "",
        progress = 0f,
        watched = watched,
        unwatched = 0uL,
    )

@Composable
fun DetailScreen(
    server: String,
    key: String,
    refresh: Int,
    open: (MediaCard) -> Unit,
    openRow: (MediaRow) -> Unit,
    onChanged: () -> Unit,
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    val play = LocalPlay.current
    var detail by remember { mutableStateOf<Detail?>(null) }
    var error by remember { mutableStateOf<String?>(null) }
    var season by remember { mutableStateOf<String?>(null) }
    var episodes by remember { mutableStateOf<List<MediaCard>>(emptyList()) }
    var episodesOf by remember { mutableStateOf<String?>(null) }
    var related by remember { mutableStateOf<List<MediaRow>>(emptyList()) }
    val focus = remember { FocusRequester() }
    LaunchedEffect(refresh) {
        if (detail == null) io { runCatching { core.detail(server, key, true) } }.onSuccess { detail = it }
        io { runCatching { core.detail(server, key, false) } }
            .onSuccess {
                detail = it
                error = null
            }
            .onFailure { if (detail == null) error = it.message }
    }
    val d = detail
    val chosen = season ?: d?.season
    LaunchedEffect(d?.key) {
        if (d != null && d.kind in setOf("movie", "show"))
            io { runCatching { core.related(server, d.key) } }.onSuccess { related = it }
    }
    LaunchedEffect(chosen, refresh) {
        if (chosen == null) return@LaunchedEffect
        io { runCatching { core.episodes(server, chosen, true) } }
            .onSuccess { episodes = it }
            .onFailure { if (episodesOf != chosen) episodes = emptyList() }
        episodesOf = chosen
        io { runCatching { core.episodes(server, chosen, false) } }.onSuccess { episodes = it }
    }
    val touch = LocalTouch.current
    LaunchedEffect(d != null) { if (d != null && !touch) runCatching { focus.requestFocus() } }
    if (d == null) return Message(error ?: "Loading…")
    val library = remember(d.server) { d.server != discoverId() }
    fun setWatched(on: Boolean) =
        scope.launch {
            val result = io {
                runCatching { core.menuAction(d.server, d.key, if (on) Action.MARK_WATCHED else Action.MARK_UNWATCHED) }
            }
            Toast.makeText(context, result.getOrElse { "Could not update: ${it.message}" }, Toast.LENGTH_SHORT).show()
            if (result.isSuccess) onChanged()
        }
    val window = LocalWindowInfo.current.containerSize
    val portrait = touch && window.width < window.height
    val sideInfo = d.info.isNotEmpty() && (!touch || with(LocalDensity.current) { window.width.toDp() } >= TV_WIDTH)
    val edge = if (portrait) Dims.gutter else Dims.edge
    val screenH = with(LocalDensity.current) { window.height.toDp() }
    var hint by remember { mutableStateOf("") }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = 48.dp)) {
        if (portrait)
            item(key = "hero") {
                Column(Modifier.padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                    Box(Modifier.fillMaxWidth().aspectRatio(16f / 9f)) {
                        AsyncImage(
                            model = d.backdrop,
                            contentDescription = null,
                            contentScale = ContentScale.Crop,
                            modifier = Modifier.fillMaxSize(),
                        )
                        Box(
                            Modifier.fillMaxSize()
                                .background(Brush.verticalGradient(0.4f to Color.Transparent, 1f to Palette.bg))
                        )
                    }
                    Column(Modifier.padding(horizontal = edge), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                        if (d.eyebrow.isNotEmpty())
                            Text(d.eyebrow, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, color = Palette.muted)
                        Title(d)
                        Badges(d)
                        if (d.playKey.isNotEmpty())
                            PlayButton(d.playLabel, d.progress, Modifier.padding(top = 6.dp).fillMaxWidth()) {
                                play(d.card(d.playKey))
                            }
                        Row(
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            RoundActions(d, library, {}, Modifier, { setWatched(!d.watched) })
                            Spacer(Modifier.weight(1f))
                            MediaLine(d)
                        }
                        if (d.info.isNotEmpty())
                            Column(Modifier.padding(top = 6.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                                InfoRows(d)
                            }
                    }
                }
            }
        else
            item(key = "hero") {
                Box(Modifier.fillMaxWidth().heightIn(min = screenH * 0.62f)) {
                    AsyncImage(
                        model = d.backdrop,
                        contentDescription = null,
                        contentScale = ContentScale.Crop,
                        modifier = Modifier.fillMaxWidth().height(screenH * 0.86f),
                    )
                    Box(
                        Modifier.fillMaxWidth()
                            .height(screenH * 0.86f)
                            .background(
                                Brush.verticalGradient(0f to Palette.bg.copy(alpha = 0.25f), 0.94f to Palette.bg)
                            )
                    )
                    Box(
                        Modifier.fillMaxWidth()
                            .height(screenH * 0.86f)
                            .background(
                                Brush.horizontalGradient(
                                    0f to Palette.bg.copy(alpha = 0.6f),
                                    0.55f to Color.Transparent,
                                )
                            )
                    )
                    if (sideInfo) {
                        Column(
                            Modifier.align(Alignment.TopEnd)
                                .padding(top = 40.dp, end = Dims.edge)
                                .width(260.dp)
                                .background(Palette.bg.copy(alpha = 0.6f), RoundedCornerShape(12.dp))
                                .padding(16.dp),
                            verticalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            InfoRows(d)
                        }
                    }
                    Column(
                        Modifier.align(Alignment.BottomStart)
                            .padding(start = Dims.edge, end = Dims.edge, top = 48.dp, bottom = 16.dp)
                            .fillMaxWidth(if (sideInfo) 0.68f else 1f),
                        verticalArrangement = Arrangement.spacedBy(10.dp),
                    ) {
                        if (d.eyebrow.isNotEmpty())
                            Text(d.eyebrow, fontSize = 16.sp, fontWeight = FontWeight.SemiBold, color = Palette.muted)
                        Title(d)
                        Badges(d)
                        Row(
                            Modifier.padding(top = 6.dp),
                            verticalAlignment = Alignment.CenterVertically,
                            horizontalArrangement = Arrangement.spacedBy(10.dp),
                        ) {
                            if (d.playKey.isNotEmpty())
                                PlayButton(d.playLabel, d.progress, Modifier.focusRequester(focus)) {
                                    play(d.card(d.playKey))
                                }
                            RoundActions(
                                d,
                                library,
                                { hint = it },
                                if (d.playKey.isEmpty()) Modifier.focusRequester(focus) else Modifier,
                                { setWatched(!d.watched) },
                            )
                            Text(hint, fontSize = 13.sp, color = Palette.muted, modifier = Modifier.weight(1f))
                            MediaLine(d)
                        }
                        if (!sideInfo && d.info.isNotEmpty())
                            Column(Modifier.padding(top = 6.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
                                InfoRows(d)
                            }
                    }
                }
            }
        if (d.summary.isNotEmpty())
            item(key = "overview") {
                Column(Modifier.padding(horizontal = edge)) {
                    SectionTitle("Overview", top = 0.dp)
                    Text(
                        d.summary,
                        fontSize = 16.sp,
                        lineHeight = 25.sp,
                        color = Palette.text,
                        modifier = Modifier.widthIn(max = 900.dp),
                    )
                }
            }
        if (d.kind == "show" && d.seasons.isNotEmpty()) {
            item(key = "seasons") {
                Column(Modifier.padding(horizontal = edge)) {
                    SectionTitle("Episodes")
                    val labels = d.seasons.map { it.title }
                    val selected = d.seasons.indexOfFirst { it.key == chosen }
                    val pick = { i: Int -> season = d.seasons[i].key }
                    if (touch) Chips(labels, selected, pick)
                    else Box(Modifier.horizontalScroll(rememberScrollState())) { Chips(labels, selected, pick) }
                }
            }
            items(episodes, key = { "ep/${it.key}" }) { EpisodeRow(it, edge) { play(it) } }
        }
        if (d.cast.isNotEmpty()) item(key = "cast") { Shelf("Cast", edge) { items(d.cast) { CastCard(it) } } }
        if (d.extras.isNotEmpty())
            item(key = "extras") {
                Shelf("Trailers & Extras", edge) {
                    items(d.extras, key = { it.key }) { MediaTile(it, wide = true) { play(it) } }
                }
            }
        itemsIndexed(related, key = { i, _ -> "related/$i" }) { _, row ->
            Shelf(row.title, edge) {
                items(row.cards, key = { "${it.server}/${it.key}" }) { MediaTile(it, wide = false) { open(it) } }
                if (row.key.isNotEmpty()) item(key = "see-all") { SeeAll(wide = false, dim = false) { openRow(row) } }
            }
        }
    }
}

@Composable
private fun Title(d: Detail) {
    var failed by remember(d.logo) { mutableStateOf(false) }
    if (d.logo != null && !failed) {
        AsyncImage(
            model = d.logo,
            contentDescription = d.title,
            contentScale = ContentScale.Fit,
            alignment = Alignment.CenterStart,
            onError = { failed = true },
            modifier = Modifier.padding(bottom = 4.dp).sizeIn(maxWidth = 360.dp, maxHeight = 120.dp),
        )
    } else {
        Text(
            d.title,
            fontSize = 36.sp,
            lineHeight = 42.sp,
            fontWeight = FontWeight.Bold,
            color = Palette.text,
            maxLines = 2,
            overflow = TextOverflow.Ellipsis,
        )
    }
}

@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun Badges(d: Detail) {
    FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
        d.chips.forEach { Badge { Text(it) } }
        if (d.ratings.isNotEmpty())
            Badge {
                d.ratings.forEach {
                    Row(
                        verticalAlignment = Alignment.CenterVertically,
                        horizontalArrangement = Arrangement.spacedBy(5.dp),
                    ) {
                        SourceTag(it.source)
                        Text(it.value)
                    }
                }
            }
    }
    if (d.genres.isNotEmpty())
        FlowRow(horizontalArrangement = Arrangement.spacedBy(6.dp)) { d.genres.forEach { Badge { Text(it) } } }
}

@Composable
private fun InfoRows(d: Detail) {
    d.info.forEach {
        Column {
            Text(it.label, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, color = Palette.tint(0.6f))
            Text(it.value, fontSize = 14.sp, color = Palette.text)
        }
    }
}

@Composable
private fun MediaLine(d: Detail) {
    if (d.mediaLine.isNotEmpty())
        Text(d.mediaLine, fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = Palette.tint(0.8f))
}

@Composable
private fun RoundActions(
    d: Detail,
    library: Boolean,
    onHint: (String) -> Unit,
    modifier: Modifier = Modifier,
    toggleWatched: () -> Unit,
) {
    val play = LocalPlay.current
    val menu = LocalCardMenu.current
    d.trailerKey?.let { t -> RoundAction(Glyph.Movie, "Play trailer", onHint) { play(d.card(t)) } }
    if (library)
        RoundAction(
            if (d.watched) Glyph.Remove else Glyph.Check,
            if (d.watched) "Mark as unwatched" else "Mark as watched",
            onHint,
            onClick = toggleWatched,
        )
    RoundAction(Glyph.MoreVert, "More", onHint, modifier) { menu?.invoke(d.card()) }
}

@Composable
private fun SectionTitle(text: String, top: Dp = 16.dp) {
    Text(
        text,
        fontSize = 22.sp,
        fontWeight = FontWeight.Bold,
        color = Palette.text,
        modifier = Modifier.padding(top = top, bottom = 8.dp),
    )
}

@Composable
private fun Badge(content: @Composable () -> Unit) {
    Row(
        Modifier.height(26.dp).background(Palette.bg.copy(alpha = 0.55f), CircleShape).padding(horizontal = 10.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(12.dp),
    ) {
        CompositionLocalProvider(
            LocalContentColor provides Palette.text,
            LocalTextStyle provides TextStyle(fontSize = 13.sp, fontWeight = FontWeight.SemiBold),
        ) {
            content()
        }
    }
}

@Composable
private fun SourceTag(source: RatingSource) {
    @Composable
    fun tag(label: String, bg: Color, fg: Color) =
        Box(
            Modifier.height(16.dp).background(bg, RoundedCornerShape(3.dp)).padding(horizontal = 4.dp),
            contentAlignment = Alignment.Center,
        ) {
            Text(label, fontSize = 10.sp, fontWeight = FontWeight.ExtraBold, color = fg)
        }
    when (source) {
        RatingSource.IMDB -> tag("IMDb", Color(0xFFF5C518), Color.Black)
        RatingSource.TMDB -> tag("TMDB", Color(0xFF01B4E4), Color(0xFF0D253F))
        RatingSource.CRITICS -> Text("🍅", fontSize = 13.sp)
        RatingSource.AUDIENCE -> Text("🍿", fontSize = 13.sp)
        RatingSource.STARS -> GlyphIcon(Glyph.Star, 14.dp, Palette.text)
    }
}

@Composable
private fun PlayButton(label: String, progress: Float, modifier: Modifier, onClick: () -> Unit) {
    val content: @Composable BoxScope.() -> Unit = {
        Row(
            Modifier.fillMaxHeight().padding(horizontal = 22.dp),
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp),
        ) {
            GlyphIcon(Glyph.Play, 22.dp, Palette.bg)
            if (progress > 0f) {
                Box(Modifier.width(40.dp).height(4.dp).background(Palette.bg.copy(alpha = 0.25f), CircleShape)) {
                    Box(Modifier.fillMaxWidth(progress).height(4.dp).background(Palette.bg, CircleShape))
                }
            }
            Text(label, fontSize = 15.sp, fontWeight = FontWeight.SemiBold)
        }
    }
    if (LocalTouch.current)
        return Tap(onClick, modifier.height(48.dp), CircleShape, Palette.text, Palette.bg, content = content)
    Surface(
        onClick = onClick,
        modifier = modifier.height(48.dp),
        shape = ClickableSurfaceDefaults.shape(CircleShape),
        colors =
            ClickableSurfaceDefaults.colors(
                containerColor = Palette.text,
                contentColor = Palette.bg,
                focusedContainerColor = Palette.text,
                focusedContentColor = Palette.bg,
            ),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
        border =
            ClickableSurfaceDefaults.border(
                focusedBorder = Border(BorderStroke(2.dp, Palette.text), inset = (-4).dp, shape = CircleShape)
            ),
        content = content,
    )
}

@Composable
private fun RoundAction(
    glyph: Glyph,
    label: String,
    onHint: (String) -> Unit,
    modifier: Modifier = Modifier,
    onClick: () -> Unit,
) {
    if (LocalTouch.current)
        return Tap(
            onClick,
            modifier.size(48.dp).semantics { contentDescription = label },
            CircleShape,
            Palette.tint(0.08f),
            Palette.text,
        ) {
            GlyphIcon(glyph, 24.dp, Palette.text)
        }
    Surface(
        onClick = onClick,
        modifier =
            modifier.size(48.dp).onFocusChanged { if (it.isFocused) onHint(label) else if (!it.hasFocus) onHint("") },
        shape = ClickableSurfaceDefaults.shape(CircleShape),
        colors =
            ClickableSurfaceDefaults.colors(
                containerColor = Palette.tint(0.08f),
                contentColor = Palette.text,
                focusedContainerColor = Palette.tint(0.16f),
                focusedContentColor = Palette.text,
            ),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
        border =
            ClickableSurfaceDefaults.border(
                focusedBorder = Border(BorderStroke(2.dp, Palette.text), inset = (-3).dp, shape = CircleShape)
            ),
    ) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) { GlyphIcon(glyph, 24.dp, Palette.text) }
    }
}

@Composable
private fun EpisodeRow(card: MediaCard, edge: Dp, onClick: () -> Unit) {
    val menu = LocalCardMenu.current
    val look = LocalAppearance.current
    val onLongClick = menu?.let { { it(card) } }
    val modifier = Modifier.padding(horizontal = edge - 6.dp, vertical = 2.dp).fillMaxWidth()
    val content: @Composable BoxScope.() -> Unit = {
        Row(Modifier.padding(6.dp), horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(
                Modifier.width(160.dp).aspectRatio(16f / 9f).clip(RoundedCornerShape(6.dp)).background(Palette.surface)
            ) {
                AsyncImage(
                    model = card.image,
                    contentDescription = card.title,
                    contentScale = ContentScale.Crop,
                    modifier = Modifier.fillMaxSize(),
                )
                if (card.watched && look.watchedIndicators) {
                    Box(
                        Modifier.align(Alignment.TopEnd)
                            .padding(3.dp)
                            .size(18.dp)
                            .background(Palette.text, CircleShape),
                        contentAlignment = Alignment.Center,
                    ) {
                        GlyphIcon(Glyph.Check, 12.dp, Palette.bg)
                    }
                }
                if (card.progress > 0f && !card.watched) {
                    Box(
                        Modifier.align(Alignment.BottomCenter)
                            .padding(6.dp)
                            .fillMaxWidth()
                            .height(3.dp)
                            .background(Palette.surface, CircleShape)
                    ) {
                        Box(Modifier.fillMaxWidth(card.progress).height(3.dp).background(Palette.text, CircleShape))
                    }
                }
            }
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(4.dp)) {
                Text(
                    card.title,
                    fontSize = 15.sp,
                    fontWeight = FontWeight.Bold,
                    maxLines = 1,
                    overflow = TextOverflow.Ellipsis,
                )
                if (card.summary.isNotEmpty())
                    Text(
                        card.summary,
                        fontSize = 13.sp,
                        lineHeight = 17.sp,
                        color = Palette.muted,
                        maxLines = 3,
                        overflow = TextOverflow.Ellipsis,
                    )
                if (card.meta.isNotEmpty()) Text(card.meta, fontSize = 12.sp, color = Palette.muted)
            }
        }
    }
    if (LocalTouch.current) return Tap(onClick, modifier, rowShape, Palette.bg, Palette.text, onLongClick, content)
    Surface(
        onClick = onClick,
        onLongClick = onLongClick,
        modifier = modifier,
        shape = ClickableSurfaceDefaults.shape(rowShape),
        colors =
            ClickableSurfaceDefaults.colors(
                containerColor = Palette.bg,
                contentColor = Palette.text,
                focusedContainerColor = Palette.tint(0.08f),
                focusedContentColor = Palette.text,
            ),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
        border =
            ClickableSurfaceDefaults.border(focusedBorder = Border(BorderStroke(2.dp, Palette.text), shape = rowShape)),
        content = content,
    )
}

@Composable
private fun Shelf(title: String, edge: Dp, content: LazyListScope.() -> Unit) {
    Column {
        Box(Modifier.padding(horizontal = edge)) { SectionTitle(title) }
        LazyRow(
            contentPadding = PaddingValues(horizontal = edge, vertical = 4.dp),
            horizontalArrangement = Arrangement.spacedBy(LocalAppearance.current.gap),
            content = content,
        )
    }
}

@Composable
private fun CastCard(p: CastMember) {
    val shape = RoundedCornerShape(8.dp)
    Column(Modifier.width(120.dp)) {
        val photo: @Composable BoxScope.() -> Unit = {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                if (p.image != null)
                    AsyncImage(
                        model = p.image,
                        contentDescription = p.name,
                        contentScale = ContentScale.Crop,
                        modifier = Modifier.fillMaxSize(),
                    )
                else GlyphIcon(Glyph.Person, 48.dp, Palette.muted)
            }
        }
        if (LocalTouch.current) Box(Modifier.size(120.dp).clip(shape).background(Palette.surface), content = photo)
        else
            Surface(
                onClick = {},
                modifier = Modifier.size(120.dp),
                shape = ClickableSurfaceDefaults.shape(shape),
                colors =
                    ClickableSurfaceDefaults.colors(
                        containerColor = Palette.surface,
                        focusedContainerColor = Palette.surface,
                    ),
                scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
                border =
                    ClickableSurfaceDefaults.border(
                        focusedBorder = Border(BorderStroke(2.5.dp, Palette.text), shape = shape)
                    ),
                content = photo,
            )
        Text(
            p.name,
            fontSize = 13.sp,
            fontWeight = FontWeight.SemiBold,
            color = Palette.text,
            maxLines = 1,
            overflow = TextOverflow.Ellipsis,
            modifier = Modifier.padding(top = 6.dp),
        )
        Text(p.role, fontSize = 12.sp, color = Palette.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
    }
}
