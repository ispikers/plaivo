package xyz.spikers.plaivo

import android.Manifest
import android.app.Activity
import android.app.Application
import android.content.Context
import android.content.pm.PackageManager
import android.os.Build
import android.text.format.Formatter
import android.widget.Toast
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
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.lazy.grid.GridCells
import androidx.compose.foundation.lazy.grid.GridItemSpan
import androidx.compose.foundation.lazy.grid.LazyVerticalGrid
import androidx.compose.foundation.lazy.grid.itemsIndexed
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberUpdatedState
import androidx.compose.runtime.setValue
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.compose.ui.window.Dialog
import androidx.tv.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.channels.Channel
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.Download
import uniffi.plaivo_ffi.DownloadListener
import uniffi.plaivo_ffi.DownloadScope
import uniffi.plaivo_ffi.DownloadState
import java.io.File

object DownloadQueue : DownloadListener {
    private val signal = Channel<Unit>(Channel.CONFLATED)
    val entries = MutableStateFlow<List<Download>>(emptyList())
    lateinit var root: File
        private set

    override fun changed() {
        signal.trySend(Unit)
    }

    fun start(app: Application) {
        root = app.getExternalFilesDir("downloads") ?: app.filesDir.resolve("downloads")
        DownloadService.createChannel(app)
        background.launch {
            core.startDownloads(root.absolutePath, this@DownloadQueue)
            for (unit in signal) {
                val list = core.downloads()
                entries.value = list
                DownloadService.sync(app, list)
            }
        }
    }
}

fun fmtSize(bytes: ULong): String = Formatter.formatShortFileSize(App.instance, bytes.toLong())

private fun Download.detail(available: Boolean): String {
    val size = if (total > 0uL) "${fmtSize(bytes)} of ${fmtSize(total)}" else ""
    val status = when (state) {
        DownloadState.DOWNLOADING -> listOf(if (total > 0uL) "${bytes * 100uL / total}%" else "Starting", size).filter(String::isNotEmpty).joinToString(" · ")
        DownloadState.QUEUED -> if (available) "Queued" else "Server unavailable"
        DownloadState.PAUSED -> listOf("Paused", size).filter(String::isNotEmpty).joinToString(" · ")
        DownloadState.FAILED -> "Failed: $error"
        DownloadState.DONE -> "Downloaded"
    }
    return listOf(subtitle, status).filter(String::isNotEmpty).joinToString(" · ")
}

@Composable
fun DownloadsScreen(play: (MediaCard) -> Unit) {
    val entries by DownloadQueue.entries.collectAsState()
    val queue = entries.filter { it.state != DownloadState.DONE }
    val done = entries.filter { it.state == DownloadState.DONE }.map { "${it.server}/${it.key}/${it.watched}" }
    val library by produceState(emptyList<MediaCard>(), done) { value = io { core.offlineLibrary() } }
    val free = remember(done) { DownloadQueue.root.usableSpace.toULong() }
    val servers = remember { core.servers().map { it.id }.toSet() }
    val keys = queue.map { "q/${it.server}/${it.key}" } + library.map { "${it.server}/${it.key}" }
    val latestKeys by rememberUpdatedState(keys)
    val requesters = remember { mutableMapOf<String, FocusRequester>() }
    val header = remember { FocusRequester() }
    var focused by remember { mutableStateOf<Pair<String, Int>?>(null) }
    LaunchedEffect(keys) {
        requesters.keys.retainAll(keys.toSet())
        val (_, index) = focused?.takeIf { it.first !in keys } ?: return@LaunchedEffect
        focused = null
        val next = keys.getOrNull(index.coerceAtMost(keys.lastIndex))?.let { requesters[it] }
        if (next == null || runCatching { next.requestFocus() }.isFailure) runCatching { header.requestFocus() }
    }
    fun Modifier.tracked(key: String, index: Int) = focusRequester(requesters.getOrPut(key) { FocusRequester() }).onFocusChanged {
        if (it.hasFocus) focused = key to index
        else if (focused?.first == key && key in latestKeys) focused = null
    }
    LazyVerticalGrid(
        columns = GridCells.FixedSize(Dims.poster * LocalCardScale.current + 14.dp),
        modifier = Modifier.fillMaxSize(),
        contentPadding = PaddingValues(start = Dims.edge - 7.dp, end = Dims.edge - 7.dp, top = 28.dp, bottom = 28.dp),
        verticalArrangement = Arrangement.spacedBy(18.dp),
    ) {
        item(span = { GridItemSpan(maxLineSpan) }) {
            Column(Modifier.padding(start = 7.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
                Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                    Heading("Downloads")
                    val status = if (entries.isEmpty()) "Nothing downloaded yet. Hold OK on a poster to download it." else "${library.size} downloaded · ${queue.size} in queue"
                    Text("$status · ${fmtSize(free)} free", fontSize = 13.sp, color = Palette.muted)
                }
                Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                    val running = queue.any { it.state == DownloadState.QUEUED || it.state == DownloadState.DOWNLOADING }
                    val stopped = queue.any { it.state == DownloadState.PAUSED || it.state == DownloadState.FAILED }
                    ActionButton("Pause all", { background.launch { core.pauseAllDownloads() } }, Modifier.focusRequester(header), running)
                    ActionButton("Resume all", { background.launch { core.resumeAllDownloads() } }, enabled = stopped)
                }
            }
        }
        itemsIndexed(queue, key = { _, d -> "q/${d.server}/${d.key}" }, span = { _, _ -> GridItemSpan(maxLineSpan) }) { i, d ->
            QueueRow(d, d.server in servers, Modifier.tracked("q/${d.server}/${d.key}", i))
        }
        if (library.isNotEmpty()) {
            item(span = { GridItemSpan(maxLineSpan) }) {
                Text("Downloaded", fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = Palette.muted, modifier = Modifier.padding(start = 7.dp, top = 8.dp))
            }
        }
        itemsIndexed(library, key = { _, card -> "${card.server}/${card.key}" }) { i, card ->
            Box(Modifier.fillMaxWidth().tracked("${card.server}/${card.key}", queue.size + i), contentAlignment = Alignment.TopCenter) { MediaTile(card, wide = false) { play(card) } }
        }
    }
}

@Composable
private fun QueueRow(d: Download, available: Boolean, modifier: Modifier) {
    val running = d.state == DownloadState.QUEUED || d.state == DownloadState.DOWNLOADING
    Row(modifier.padding(horizontal = 7.dp), horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.CenterVertically) {
        SettingsRow(Glyph.Download, d.title, d.detail(available), Modifier.weight(1f), onClick = {
            background.launch { if (running) core.pauseDownload(d.server, d.key) else core.resumeDownload(d.server, d.key) }
        }) {
            if (d.total > 0uL) {
                Box(Modifier.width(120.dp).height(4.dp).background(Palette.tint(0.2f), CircleShape)) {
                    Box(Modifier.fillMaxWidth((d.bytes.toDouble() / d.total.toDouble()).toFloat().coerceIn(0f, 1f)).height(4.dp).background(Palette.text, CircleShape))
                }
            }
            Text(if (running) "Pause" else if (d.state == DownloadState.FAILED) "Retry" else "Resume", fontSize = 14.sp, color = Palette.muted)
        }
        MoveButton(Glyph.Close, true) { background.launch { core.removeDownload(d.server, d.key) } }
    }
}

@Composable
fun CardMenu(card: MediaCard, onDismiss: () -> Unit) {
    val context = LocalContext.current
    val entries by DownloadQueue.entries.collectAsState()
    val saved = entries.any { it.server == card.server && it.key == card.key }
    val scopes = when (card.kind) {
        "movie" -> listOf("Download movie" to DownloadScope.ITEM)
        "episode" -> listOf("Download episode" to DownloadScope.ITEM, "Download season" to DownloadScope.SEASON, "Download show" to DownloadScope.SHOW)
        "season" -> listOf("Download season" to DownloadScope.ITEM, "Download show" to DownloadScope.SHOW)
        "show" -> listOf("Download show" to DownloadScope.ITEM)
        else -> emptyList()
    }
    val first = remember { FocusRequester() }
    val shape = RoundedCornerShape(16.dp)
    Dialog(onDismissRequest = onDismiss) {
        Column(
            Modifier.width(460.dp).background(Palette.bg, shape).border(1.dp, Palette.outline, shape).padding(20.dp),
            verticalArrangement = Arrangement.spacedBy(10.dp),
        ) {
            Text(card.title, fontSize = 18.sp, fontWeight = FontWeight.Bold, color = Palette.text, maxLines = 1)
            if (card.subtitle.isNotEmpty()) Text(card.subtitle, fontSize = 13.sp, color = Palette.muted, maxLines = 1)
            when {
                saved -> SettingsRow(Glyph.Close, "Remove download", "Deletes it from this device", Modifier.focusRequester(first), onClick = {
                    background.launch { core.removeDownload(card.server, card.key) }
                    onDismiss()
                })
                scopes.isEmpty() -> Text("This can't be downloaded", fontSize = 14.sp, color = Palette.muted)
                else -> scopes.forEachIndexed { i, (label, scope) ->
                    SettingsRow(Glyph.Download, label, "", if (i == 0) Modifier.focusRequester(first) else Modifier, onClick = {
                        download(context, card, scope)
                        onDismiss()
                    })
                }
            }
        }
        LaunchedEffect(Unit) { runCatching { first.requestFocus() } }
    }
}

private fun download(context: Context, card: MediaCard, scope: DownloadScope) {
    if (Build.VERSION.SDK_INT >= 33 && context.checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) != PackageManager.PERMISSION_GRANTED) {
        (context as? Activity)?.requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 0)
    }
    background.launch {
        val error = runCatching { core.download(card.server, card.key, scope) }.exceptionOrNull()
        withContext(Dispatchers.Main) {
            Toast.makeText(context, error?.let { "Could not download: ${it.message}" } ?: "Added to Downloads", Toast.LENGTH_SHORT).show()
        }
    }
}
