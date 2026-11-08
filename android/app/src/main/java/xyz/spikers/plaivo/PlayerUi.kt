@file:OptIn(UnstableApi::class)

package xyz.spikers.plaivo

import androidx.annotation.OptIn
import androidx.media3.common.util.UnstableApi
import android.text.format.DateFormat
import androidx.activity.compose.BackHandler
import androidx.compose.animation.AnimatedVisibility
import androidx.compose.animation.fadeIn
import androidx.compose.animation.fadeOut
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEvent
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onKeyEvent
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.viewinterop.AndroidView
import androidx.media3.common.C
import androidx.media3.ui.CaptionStyleCompat
import androidx.media3.ui.SubtitleView
import androidx.media3.ui.PlayerView
import androidx.tv.material3.Border
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import kotlinx.coroutines.delay
import java.util.Date
import kotlin.math.ceil
import uniffi.plaivo_ffi.Settings
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.SkipMode
import uniffi.plaivo_ffi.WatchConnection

private const val CONTROLS_FOR_MS = 3000L
private const val HOLD_EVERY = 5
private const val HINT_MS = 1200L

private enum class Panel(val title: String, val parent: Panel? = null) {
    Settings("Playback settings"),
    Quality("Quality", Settings),
    Speed("Playback Speed", Settings),
    Info("File Info", Settings),
    Tracks("Audio & subtitles"),
    Queue("Queue"),
}

private enum class Target { Play, Timeline, Skip }

private val white = Color.White

@Composable
fun PlayerScreen(player: PlayerActivity) {
    val exo = player.exo
    val p = player.playback
    val context = LocalContext.current
    var position by remember { mutableLongStateOf(0L) }
    var buffered by remember { mutableLongStateOf(0L) }
    var duration by remember { mutableLongStateOf(0L) }
    var paused by remember { mutableStateOf(false) }
    var now by remember { mutableLongStateOf(System.currentTimeMillis()) }
    var controls by remember { mutableStateOf(true) }
    var lastInput by remember { mutableLongStateOf(0L) }
    var panel by remember { mutableStateOf<Panel?>(null) }
    var target by remember { mutableStateOf<Target?>(Target.Play) }
    var swallow by remember { mutableStateOf<Key?>(null) }
    var hint by remember { mutableStateOf<Pair<Glyph, String>?>(null) }
    var upNext by remember { mutableStateOf<Double?>(null) }
    val small = player.prefs.seekSmall.toLong() * 1000
    val large = player.prefs.seekLarge.toLong() * 1000
    val root = remember { FocusRequester() }
    val playButton = remember { FocusRequester() }
    val timeline = remember { FocusRequester() }
    val skipButton = remember { FocusRequester() }
    val room = Watch.state?.takeIf { Watch.inRoom }

    LaunchedEffect(Unit) {
        while (true) {
            position = exo.currentPosition.coerceAtLeast(0)
            buffered = exo.bufferedPosition.coerceAtLeast(0)
            duration = exo.duration.takeIf { it > 0 } ?: (player.playback?.durationMs?.toLong() ?: 0L)
            paused = !exo.playWhenReady
            now = System.currentTimeMillis()
            player.feedWatch()
            if (!paused) {
                player.autoSkip(position)?.let { hint = Glyph.FastForward to it }
                upNext = player.upNextIn(position, duration)
                if ((upNext ?: 1.0) <= 0.0) player.playNext()
            } else if (player.upNextCancelled) {
                upNext = null
            }
            delay(250)
        }
    }
    val skip = p?.markers?.withIndex()?.firstOrNull { (i, m) ->
        position >= m.startMs.toLong() && position < m.endMs.toLong() - 1000 && (m.mode == SkipMode.BUTTON || (m.mode == SkipMode.AUTO && (i in player.autoSkipped || player.watching())))
    }?.value?.takeIf { upNext == null }
    val hideable = !paused && panel == null
    LaunchedEffect(controls, lastInput, hideable) {
        if (controls && hideable) {
            delay(CONTROLS_FOR_MS)
            controls = false
            runCatching { root.requestFocus() }
        }
    }
    LaunchedEffect(controls, target, skip != null) {
        val t = target ?: return@LaunchedEffect
        if (!controls) return@LaunchedEffect
        delay(16)
        runCatching {
            when (t) {
                Target.Play -> playButton.requestFocus()
                Target.Timeline -> timeline.requestFocus()
                Target.Skip -> if (skip != null) skipButton.requestFocus() else playButton.requestFocus()
            }
        }
        target = null
    }
    LaunchedEffect(hint) {
        if (hint != null) {
            delay(HINT_MS)
            hint = null
        }
    }
    fun show(focus: Target) {
        controls = true
        target = focus
    }
    BackHandler {
        val open = panel
        when {
            open != null -> {
                panel = open.parent
                if (open.parent == null) target = Target.Play
            }
            upNext != null && !controls -> player.cancelUpNext()
            controls && !paused -> {
                controls = false
                runCatching { root.requestFocus() }
            }
            else -> player.finish()
        }
    }

    Box(
        Modifier
            .fillMaxSize()
            .background(Color.Black)
            .onPreviewKeyEvent { e ->
                lastInput = System.nanoTime()
                if (e.type == KeyEventType.KeyUp) {
                    if (swallow == e.key) {
                        swallow = null
                        return@onPreviewKeyEvent true
                    }
                    return@onPreviewKeyEvent false
                }
                if (e.type != KeyEventType.KeyDown) return@onPreviewKeyEvent false
                when (e.key) {
                    Key.MediaPlayPause, Key.MediaPlay, Key.MediaPause -> {
                        if (e.key == Key.MediaPlayPause) player.togglePause() else player.setPlaying(e.key == Key.MediaPlay)
                        show(Target.Play)
                        return@onPreviewKeyEvent true
                    }
                    Key.MediaFastForward -> { player.seekBy(large); return@onPreviewKeyEvent true }
                    Key.MediaRewind -> { player.seekBy(-large); return@onPreviewKeyEvent true }
                    Key.MediaNext -> { player.neighbor(1)?.let { player.play(it.server, it.key) }; return@onPreviewKeyEvent true }
                    Key.MediaPrevious -> { player.neighbor(-1)?.let { player.play(it.server, it.key) }; return@onPreviewKeyEvent true }
                }
                if (controls) return@onPreviewKeyEvent false
                swallow = e.key
                val step = seekStep(e, small, large)
                when (e.key) {
                    Key.DirectionCenter, Key.Enter, Key.NumPadEnter -> when {
                        upNext != null -> player.playNext()
                        skip != null -> player.seekTo(skip.endMs.toLong())
                        else -> show(Target.Play)
                    }
                    Key.DirectionLeft -> { if (step != 0L) player.seekBy(-step); show(Target.Timeline) }
                    Key.DirectionRight -> { if (step != 0L) player.seekBy(step); show(Target.Timeline) }
                    Key.DirectionUp, Key.DirectionDown -> show(Target.Play)
                    else -> {
                        swallow = null
                        return@onPreviewKeyEvent false
                    }
                }
                true
            }
            .focusRequester(root)
            .focusable(),
    ) {
        AndroidView(
            factory = { ctx ->
                PlayerView(ctx).apply {
                    useController = false
                    isFocusable = false
                    keepScreenOn = true
                    setShowBuffering(PlayerView.SHOW_BUFFERING_WHEN_PLAYING)
                    subtitleView?.let { styleSubtitles(it, player.prefs) }
                    this.player = exo
                }
            },
            update = { it.resizeMode = player.scale.resize },
            modifier = Modifier.fillMaxSize(),
        )

        AnimatedVisibility(controls, enter = fadeIn(), exit = fadeOut()) {
            Box(Modifier.fillMaxSize()) {
                Box(Modifier.fillMaxWidth().height(140.dp).background(Brush.verticalGradient(listOf(Color.Black.copy(alpha = 0.75f), Color.Transparent))))
                Box(Modifier.align(Alignment.BottomCenter).fillMaxWidth().height(220.dp).background(Brush.verticalGradient(listOf(Color.Transparent, Color.Black.copy(alpha = 0.75f)))))
                Row(Modifier.fillMaxWidth().padding(horizontal = 40.dp, vertical = 24.dp), verticalAlignment = Alignment.Top) {
                    Column(Modifier.weight(1f)) {
                        Text(p?.title ?: "", fontSize = 18.sp, fontWeight = FontWeight.Bold, color = white, maxLines = 1, overflow = TextOverflow.Ellipsis)
                        if (!p?.subtitle.isNullOrEmpty()) Text(p?.subtitle ?: "", fontSize = 14.sp, color = white.copy(alpha = 0.7f), maxLines = 1, overflow = TextOverflow.Ellipsis)
                    }
                    room?.let { WatchPill(it, Modifier.padding(end = 16.dp)) }
                    Text(DateFormat.getTimeFormat(context).format(Date(now)), fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = white)
                }
                Column(Modifier.align(Alignment.BottomCenter).fillMaxWidth().padding(start = 40.dp, end = 40.dp, bottom = 20.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) {
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                        Text(fmtTime(position), fontSize = 13.sp, color = white, modifier = Modifier.widthIn(min = 48.dp))
                        Timeline(position, buffered, duration, small, large, Modifier.weight(1f).focusRequester(timeline), onSeek = { player.seekBy(it); position = (position + it).coerceIn(0, duration) }, onToggle = player::togglePause)
                        Text("-${fmtTime((duration - position).coerceAtLeast(0))}", fontSize = 13.sp, color = white, modifier = Modifier.widthIn(min = 52.dp))
                    }
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(6.dp)) {
                        val episodic = player.queue.isNotEmpty() && player.neighbor(0) != null
                        val prev = player.neighbor(-1)
                        val next = player.neighbor(1)
                        if (episodic) Control(Glyph.SkipPrevious, enabled = prev != null) { prev?.let { player.play(it.server, it.key) } }
                        Control(Glyph.Replay, badge = "${small / 1000}") { player.seekBy(-small) }
                        Control(if (paused) Glyph.Play else Glyph.Pause, size = 52.dp, icon = 30.dp, modifier = Modifier.focusRequester(playButton)) { player.togglePause() }
                        Control(Glyph.Forward, badge = "${small / 1000}") { player.seekBy(small) }
                        if (episodic) Control(Glyph.SkipNext, enabled = next != null) { next?.let { player.play(it.server, it.key) } }
                        Control(Glyph.Stop) { player.finish() }
                        if (duration > 0) {
                            val ends = now + ((duration - position) / player.speed).toLong()
                            Text("Ends at ${DateFormat.getTimeFormat(context).format(Date(ends))}", fontSize = 13.sp, color = white.copy(alpha = 0.7f), modifier = Modifier.padding(start = 12.dp))
                        }
                        Spacer(Modifier.weight(1f))
                        Control(Glyph.Tune, selected = panel?.let { it == Panel.Settings || it.parent == Panel.Settings } == true) { panel = if (panel == Panel.Settings) null else Panel.Settings }
                        Control(Glyph.Subtitles, selected = panel == Panel.Tracks) { panel = if (panel == Panel.Tracks) null else Panel.Tracks }
                        if (episodic) Control(Glyph.Queue, selected = panel == Panel.Queue) { panel = if (panel == Panel.Queue) null else Panel.Queue }
                        Control(player.scale.glyph) {
                            val s = player.cycleScale()
                            hint = s.glyph to s.label
                        }
                    }
                }
                panel?.let { open ->
                    Box(Modifier.align(Alignment.BottomEnd).padding(end = 40.dp, bottom = 128.dp)) {
                        key(open) { PanelView(open, player, onOpen = { panel = it }, onClose = { panel = null; target = Target.Play }) }
                    }
                }
            }
        }

        if (!controls && room != null && (room.status.isNotEmpty() || room.connection != WatchConnection.CONNECTED)) {
            WatchPill(room, Modifier.align(Alignment.TopEnd).padding(top = 24.dp, end = 40.dp))
        }

        if (skip != null && panel == null) {
            Surface(
                onClick = { player.seekTo(skip.endMs.toLong()) },
                modifier = Modifier.align(Alignment.BottomEnd).padding(end = 40.dp, bottom = if (controls) 128.dp else 40.dp).focusRequester(skipButton),
                shape = ClickableSurfaceDefaults.shape(CircleShape),
                colors = ClickableSurfaceDefaults.colors(containerColor = Palette.text, contentColor = Palette.bg, focusedContainerColor = Palette.text, focusedContentColor = Palette.bg),
                scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
                border = ClickableSurfaceDefaults.border(focusedBorder = Border(BorderStroke(2.dp, white), inset = 3.dp, shape = CircleShape)),
            ) {
                Row(Modifier.height(44.dp).padding(horizontal = 18.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                    GlyphIcon(Glyph.FastForward, 20.dp, Palette.bg)
                    Text(skip.label, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = Palette.bg)
                }
            }
        }

        val next = player.neighbor(1)
        val left = upNext
        if (left != null && next != null && panel == null) {
            UpNext(next, left, controls, player::playNext, player::cancelUpNext, Modifier.align(Alignment.BottomEnd).padding(end = 40.dp, bottom = if (controls) 128.dp else 40.dp))
        }

        hint?.let { (glyph, label) ->
            Row(
                Modifier.align(Alignment.TopCenter).padding(top = 32.dp).background(Color.Black.copy(alpha = 0.7f), RoundedCornerShape(12.dp)).padding(horizontal = 16.dp, vertical = 10.dp),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.spacedBy(10.dp),
            ) {
                GlyphIcon(glyph, 22.dp, white)
                Text(label, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = white)
            }
        }
    }
}

@Composable
private fun Control(glyph: Glyph, modifier: Modifier = Modifier, size: Dp = 44.dp, icon: Dp = 24.dp, badge: String? = null, enabled: Boolean = true, selected: Boolean = false, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        enabled = enabled,
        modifier = modifier.size(size).alpha(if (enabled) 1f else 0.4f),
        shape = ClickableSurfaceDefaults.shape(CircleShape),
        colors = ClickableSurfaceDefaults.colors(
            containerColor = if (selected) white.copy(alpha = 0.18f) else Color.Transparent,
            contentColor = white,
            focusedContainerColor = white.copy(alpha = 0.22f),
            focusedContentColor = white,
            disabledContainerColor = Color.Transparent,
            disabledContentColor = white,
        ),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
        border = ClickableSurfaceDefaults.border(focusedBorder = Border(BorderStroke(2.dp, white), shape = CircleShape)),
    ) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
            GlyphIcon(glyph, icon, white)
            if (badge != null) Text(badge, fontSize = 7.sp, fontWeight = FontWeight.Bold, color = white, modifier = Modifier.padding(top = 3.dp))
        }
    }
}

private fun seekStep(e: KeyEvent, small: Long, large: Long): Long {
    val repeat = e.nativeKeyEvent.repeatCount
    return if (repeat == 0) small else if (repeat % HOLD_EVERY == 0) large else 0L
}

@Composable
private fun UpNext(next: MediaCard, left: Double, controls: Boolean, onPlay: () -> Unit, onCancel: () -> Unit, modifier: Modifier) {
    Column(modifier.width(440.dp).background(Palette.surface.copy(alpha = 0.97f), RoundedCornerShape(12.dp)).padding(12.dp), verticalArrangement = Arrangement.spacedBy(10.dp)) {
        Row(horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.CenterVertically) {
            AsyncImage(model = next.image, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.width(128.dp).height(72.dp).clip(RoundedCornerShape(6.dp)).background(Palette.bg))
            Column(Modifier.weight(1f)) {
                Text("Up next in ${ceil(left).toInt().coerceAtLeast(0)}s", fontSize = 12.sp, color = Palette.muted)
                Text(next.subtitle, fontSize = 14.sp, fontWeight = FontWeight.SemiBold, color = Palette.text, maxLines = 2, overflow = TextOverflow.Ellipsis)
                if (!controls) Text("OK plays now · Back cancels", fontSize = 12.sp, color = Palette.muted)
            }
        }
        if (controls) {
            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                PanelRow(Glyph.Play, "Play now", "", Modifier.weight(1f), onClick = onPlay)
                PanelRow(Glyph.Close, "Cancel", "", Modifier.weight(1f), onClick = onCancel)
            }
        }
    }
}

@Composable
private fun Timeline(position: Long, buffered: Long, duration: Long, small: Long, large: Long, modifier: Modifier, onSeek: (Long) -> Unit, onToggle: () -> Unit) {
    var focused by remember { mutableStateOf(false) }
    Box(
        modifier
            .height(24.dp)
            .onFocusChanged { focused = it.isFocused }
            .onKeyEvent { e ->
                if (e.type != KeyEventType.KeyDown) return@onKeyEvent false
                val step = seekStep(e, small, large)
                when (e.key) {
                    Key.DirectionLeft -> { if (step != 0L) onSeek(-step); true }
                    Key.DirectionRight -> { if (step != 0L) onSeek(step); true }
                    Key.DirectionCenter, Key.Enter, Key.NumPadEnter -> { onToggle(); true }
                    else -> false
                }
            }
            .focusable(),
        contentAlignment = Alignment.Center,
    ) {
        Canvas(Modifier.fillMaxWidth().height(24.dp)) {
            val bar = if (focused) 6.dp.toPx() else 4.dp.toPx()
            val y = (size.height - bar) / 2
            val r = CornerRadius(bar / 2)
            fun frac(v: Long) = if (duration > 0) (v.toFloat() / duration).coerceIn(0f, 1f) else 0f
            drawRoundRect(white.copy(alpha = 0.2f), Offset(0f, y), Size(size.width, bar), r)
            drawRoundRect(white.copy(alpha = 0.4f), Offset(0f, y), Size(size.width * frac(buffered), bar), r)
            drawRoundRect(white, Offset(0f, y), Size(size.width * frac(position), bar), r)
            drawCircle(white, radius = if (focused) 9.dp.toPx() else 6.dp.toPx(), center = Offset(size.width * frac(position), size.height / 2))
        }
    }
}

@Composable
private fun PanelView(panel: Panel, player: PlayerActivity, onOpen: (Panel?) -> Unit, onClose: () -> Unit) {
    val first = remember { FocusRequester() }
    LaunchedEffect(Unit) {
        delay(16)
        runCatching { first.requestFocus() }
    }
    val width = when (panel) {
        Panel.Tracks -> 600.dp
        Panel.Queue -> 440.dp
        else -> 360.dp
    }
    Column(Modifier.width(width).background(Palette.surface.copy(alpha = 0.97f), RoundedCornerShape(16.dp)).padding(vertical = 10.dp)) {
        Row(Modifier.fillMaxWidth().padding(horizontal = 18.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(8.dp)) {
            if (panel.parent != null) GlyphIcon(Glyph.ChevronLeft, 20.dp, Palette.muted)
            Text(panel.title, fontSize = 15.sp, fontWeight = FontWeight.SemiBold, color = Palette.text)
        }
        when (panel) {
            Panel.Settings -> {
                val selected = player.qualities.firstOrNull { it.kbps.toInt() == player.quality }
                val source = player.playback?.source.orEmpty()
                val quality = selected?.let { "${it.label} · ${it.bitrate}" } ?: if (source.isEmpty()) "Original" else "Original · $source"
                PanelRow(Glyph.HighQuality, "Quality", quality, chevron = true, modifier = Modifier.focusRequester(first)) { onOpen(Panel.Quality) }
                PanelRow(Glyph.Speed, "Playback Speed", fmtSpeed(player.speed), chevron = true) { onOpen(Panel.Speed) }
                PanelRow(Glyph.Info, "File Info", "", chevron = true) { onOpen(Panel.Info) }
            }
            Panel.Quality -> {
                val original = player.qualities.none { it.kbps.toInt() == player.quality }
                LazyColumn(Modifier.heightIn(max = 420.dp)) {
                    item {
                        PanelRow(null, "Original", player.playback?.source.orEmpty(), selected = original, modifier = if (original) Modifier.focusRequester(first) else Modifier) {
                            player.changeQuality(0)
                            onOpen(Panel.Settings)
                        }
                    }
                    itemsIndexed(player.qualities) { _, q ->
                        val isSelected = q.kbps.toInt() == player.quality
                        PanelRow(null, q.label, q.bitrate, selected = isSelected, modifier = if (isSelected) Modifier.focusRequester(first) else Modifier) {
                            player.changeQuality(q.kbps.toInt())
                            onOpen(Panel.Settings)
                        }
                    }
                }
            }
            Panel.Speed -> SPEEDS.forEach { s ->
                PanelRow(null, fmtSpeed(s), "", selected = s == player.speed, modifier = if (s == player.speed) Modifier.focusRequester(first) else Modifier) {
                    player.changeSpeed(s)
                    onOpen(Panel.Settings)
                }
            }
            Panel.Info -> {
                val rows = remember(player.tracksVersion, player.videoDecoder) { player.fileInfo() }
                Column(Modifier.padding(horizontal = 18.dp, vertical = 8.dp).focusRequester(first).focusable(), verticalArrangement = Arrangement.spacedBy(8.dp)) {
                    rows.forEach { (k, v) ->
                        Row(horizontalArrangement = Arrangement.spacedBy(12.dp)) {
                            Text(k, fontSize = 13.sp, color = Palette.muted, modifier = Modifier.width(84.dp))
                            Text(v, fontSize = 13.sp, color = Palette.text)
                        }
                    }
                }
            }
            Panel.Tracks -> {
                val audio = remember(player.tracksVersion) { player.tracks(C.TRACK_TYPE_AUDIO) }
                val text = remember(player.tracksVersion) { player.tracks(C.TRACK_TYPE_TEXT) }
                val textOff = player.textDisabled() || text.none { it.selected }
                Row(Modifier.heightIn(max = 380.dp)) {
                    LazyColumn(Modifier.weight(1f)) {
                        item { ColumnTitle("Audio") }
                        if (audio.isEmpty()) item { Text("No audio tracks", fontSize = 13.sp, color = Palette.muted, modifier = Modifier.padding(horizontal = 18.dp)) }
                        itemsIndexed(audio) { i, t ->
                            PanelRow(null, t.label, t.detail, selected = t.selected, modifier = if (t.selected || (i == 0 && audio.none { it.selected })) Modifier.focusRequester(first) else Modifier) { player.select(C.TRACK_TYPE_AUDIO, t) }
                        }
                    }
                    LazyColumn(Modifier.weight(1f)) {
                        item { ColumnTitle("Subtitles") }
                        item { PanelRow(null, "Off", "", selected = textOff) { player.select(C.TRACK_TYPE_TEXT, null) } }
                        itemsIndexed(text) { _, t -> PanelRow(null, t.label, t.detail, selected = t.selected && !textOff) { player.select(C.TRACK_TYPE_TEXT, t) } }
                    }
                }
            }
            Panel.Queue -> {
                val current = player.queue.indexOfFirst { it.key == player.playback?.key }.coerceAtLeast(0)
                val list = rememberLazyListState(initialFirstVisibleItemIndex = (current - 1).coerceAtLeast(0))
                LazyColumn(Modifier.heightIn(max = 420.dp), state = list) {
                    itemsIndexed(player.queue, key = { _, c -> c.key }) { i, card ->
                        QueueRow(card, playing = i == current, modifier = if (i == current) Modifier.focusRequester(first) else Modifier) {
                            if (i != current) player.play(card.server, card.key)
                            onClose()
                        }
                    }
                }
            }
        }
    }
}

@Composable
private fun ColumnTitle(text: String) {
    Text(text.uppercase(), fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = Palette.muted, modifier = Modifier.padding(start = 18.dp, top = 6.dp, bottom = 4.dp))
}

@Composable
private fun PanelRow(glyph: Glyph?, label: String, detail: String, modifier: Modifier = Modifier, selected: Boolean = false, chevron: Boolean = false, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        modifier = modifier.fillMaxWidth().padding(horizontal = 8.dp),
        shape = ClickableSurfaceDefaults.shape(RoundedCornerShape(10.dp)),
        colors = ClickableSurfaceDefaults.colors(containerColor = Color.Transparent, contentColor = Palette.text, focusedContainerColor = Palette.tint(0.12f), focusedContentColor = Palette.text),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
    ) {
        Row(Modifier.fillMaxWidth().heightIn(min = 44.dp).padding(horizontal = 10.dp, vertical = 6.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            if (glyph != null) GlyphIcon(glyph, 20.dp, Palette.muted)
            Column(Modifier.weight(1f)) {
                Text(label, fontSize = 14.sp, color = Palette.text, maxLines = 1, overflow = TextOverflow.Ellipsis)
                if (detail.isNotEmpty() && !chevron) Text(detail, fontSize = 12.sp, color = Palette.muted, maxLines = 1, overflow = TextOverflow.Ellipsis)
            }
            if (chevron && detail.isNotEmpty()) Text(detail, fontSize = 13.sp, color = Palette.muted)
            if (chevron) GlyphIcon(Glyph.ChevronRight, 18.dp, Palette.muted)
            if (selected) GlyphIcon(Glyph.Check, 20.dp, Palette.text)
        }
    }
}

@Composable
private fun QueueRow(card: MediaCard, playing: Boolean, modifier: Modifier, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        modifier = modifier.fillMaxWidth().padding(horizontal = 8.dp),
        shape = ClickableSurfaceDefaults.shape(RoundedCornerShape(10.dp)),
        colors = ClickableSurfaceDefaults.colors(containerColor = if (playing) Palette.tint(0.08f) else Color.Transparent, contentColor = Palette.text, focusedContainerColor = Palette.tint(0.12f), focusedContentColor = Palette.text),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
    ) {
        Row(Modifier.fillMaxWidth().padding(8.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(12.dp)) {
            Box(Modifier.width(112.dp).height(63.dp).background(Palette.bg, RoundedCornerShape(6.dp))) {
                AsyncImage(model = card.image, contentDescription = null, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize().clip(RoundedCornerShape(6.dp)))
                if (playing) {
                    Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.5f), RoundedCornerShape(6.dp)), contentAlignment = Alignment.Center) { GlyphIcon(Glyph.Play, 24.dp, white) }
                } else if (card.watched) {
                    Box(Modifier.align(Alignment.TopEnd).padding(3.dp).size(18.dp).background(Palette.text, CircleShape), contentAlignment = Alignment.Center) { GlyphIcon(Glyph.Check, 12.dp, Palette.bg) }
                }
                if (card.progress > 0f && !card.watched && !playing) {
                    Box(Modifier.align(Alignment.BottomCenter).padding(6.dp).fillMaxWidth().height(3.dp).background(Palette.surface, CircleShape)) {
                        Box(Modifier.fillMaxWidth(card.progress).height(3.dp).background(Palette.text, CircleShape))
                    }
                }
            }
            Text(card.subtitle, fontSize = 13.sp, color = Palette.text, maxLines = 2, overflow = TextOverflow.Ellipsis, modifier = Modifier.weight(1f))
        }
    }
}

private fun fmtTime(ms: Long): String {
    val s = ms / 1000
    val (h, m, sec) = Triple(s / 3600, s % 3600 / 60, s % 60)
    return if (h > 0) "%d:%02d:%02d".format(h, m, sec) else "%d:%02d".format(m, sec)
}

private fun styleSubtitles(view: SubtitleView, s: Settings) {
    val color = (s.subtitleColor.toIntOrNull(16) ?: 0xFFFFFF) or 0xFF000000.toInt()
    val back = android.graphics.Color.argb(s.subtitleBackground.toInt().coerceIn(0, 100) * 255 / 100, 0, 0, 0)
    val edge = if (s.subtitleOutline > 0u) CaptionStyleCompat.EDGE_TYPE_OUTLINE else CaptionStyleCompat.EDGE_TYPE_NONE
    view.setStyle(CaptionStyleCompat(color, back, android.graphics.Color.TRANSPARENT, edge, android.graphics.Color.BLACK, null))
    view.setFractionalTextSize(SubtitleView.DEFAULT_TEXT_SIZE_FRACTION * s.subtitleSize.toFloat() / 100f)
    view.setBottomPaddingFraction(SubtitleView.DEFAULT_BOTTOM_PADDING_FRACTION + (100 - s.subtitlePosition.toInt().coerceAtMost(100)) / 100f)
}
