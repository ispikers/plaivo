package xyz.spikers.plaivo

import android.content.Context
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.focusable
import androidx.compose.foundation.gestures.animateScrollBy
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.itemsIndexed
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.input.key.Key
import androidx.compose.ui.input.key.KeyEventType
import androidx.compose.ui.input.key.key
import androidx.compose.ui.input.key.onPreviewKeyEvent
import androidx.compose.ui.input.key.type
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Text
import kotlinx.coroutines.launch
import org.json.JSONObject
import uniffi.plaivo_ffi.rustLicenseText
import uniffi.plaivo_ffi.rustLicenses

private class Notice(val name: String, val detail: String, val text: () -> String)

private fun detail(version: String, license: String) =
    listOf(version, license).filter { it.isNotBlank() }.joinToString(" · ")

private fun loadNotices(context: Context): List<Notice> {
    val json =
        JSONObject(context.resources.openRawResource(R.raw.aboutlibraries).bufferedReader().use { it.readText() })
    val texts = json.getJSONObject("licenses")
    val libraries = json.getJSONArray("libraries")
    val maven =
        (0 until libraries.length()).map { i ->
            val lib = libraries.getJSONObject(i)
            val ids = lib.optJSONArray("licenses")?.let { a -> (0 until a.length()).map { a.getString(it) } }.orEmpty()
            val name = lib.optString("name").ifBlank { lib.getString("uniqueId") }
            Notice(name, detail(lib.optString("artifactVersion"), ids.joinToString(" OR "))) {
                ids.joinToString("\n\n") { texts.optJSONObject(it)?.optString("content").orEmpty() }
            }
        }
    val rust =
        rustLicenses().mapIndexed { i, l ->
            Notice(l.name, detail(l.version, l.license)) { rustLicenseText(i.toUInt()) }
        }
    return (maven + rust).sortedBy { it.name.lowercase() }
}

@Composable
fun LicensesScreen(onBack: () -> Unit) {
    val context = LocalContext.current
    val notices by produceState<List<Notice>?>(null) { value = io { loadNotices(context) } }
    var open by rememberSaveable { mutableStateOf<Int?>(null) }
    var last by rememberSaveable { mutableIntStateOf(0) }
    var focused by remember { mutableStateOf(false) }
    val target = remember { FocusRequester() }
    val list = rememberLazyListState()
    val touch = LocalTouch.current
    BackHandler(enabled = focused || touch) { if (open != null) open = null else onBack() }
    LaunchedEffect(open, notices) { if (notices != null) runCatching { target.requestFocus() } }
    val items = notices ?: return
    val shown = open?.let { items.getOrNull(it) }
    Column(
        Modifier.fillMaxSize()
            .onFocusChanged { focused = it.hasFocus }
            .padding(PaddingValues(horizontal = if (touch) Dims.gutter else Dims.edge, vertical = 28.dp)),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        if (shown == null) {
            Heading("Open-source licenses")
            LazyColumn(state = list, verticalArrangement = Arrangement.spacedBy(10.dp)) {
                itemsIndexed(items) { i, n ->
                    SettingsRow(
                        null,
                        n.name,
                        n.detail,
                        if (i == last) Modifier.focusRequester(target) else Modifier,
                        onClick = {
                            last = i
                            open = i
                        },
                    ) {
                        GlyphIcon(Glyph.ChevronRight, 22.dp, Palette.muted)
                    }
                }
            }
        } else {
            Heading(shown.name)
            Text(shown.detail, fontSize = 13.sp, color = Palette.muted)
            val text by produceState("", shown) { value = io { shown.text() } }
            LicenseText(text, Modifier.focusRequester(target))
        }
    }
}

@Composable
private fun LicenseText(text: String, modifier: Modifier) {
    val scroll = rememberScrollState()
    val scope = rememberCoroutineScope()
    Box(
        modifier
            .fillMaxWidth()
            .clip(RoundedCornerShape(12.dp))
            .background(Palette.surface)
            .onPreviewKeyEvent { e ->
                val step =
                    when (e.key) {
                        Key.DirectionDown -> 1
                        Key.DirectionUp -> -1
                        else -> return@onPreviewKeyEvent false
                    }
                if (e.type == KeyEventType.KeyDown) scope.launch { scroll.animateScrollBy(step * 360f) }
                true
            }
            .focusable()
            .verticalScroll(scroll)
            .padding(20.dp)
    ) {
        Text(text, fontSize = 13.sp, color = Palette.text)
    }
}
