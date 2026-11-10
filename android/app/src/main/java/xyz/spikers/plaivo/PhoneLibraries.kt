package xyz.spikers.plaivo

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.runtime.Composable
import androidx.compose.runtime.remember
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Text
import uniffi.plaivo_ffi.Section

@Composable
fun PhoneLibraries(libraries: List<Section>, onOpen: (Section) -> Unit) {
    val groups = remember(libraries) { libraries.groupBy { it.serverName } }
    LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(bottom = Dims.gutter)) {
        item(key = "heading") { Box(Modifier.padding(Dims.gutter)) { Heading("Libraries") } }
        groups.forEach { (server, libs) ->
            if (groups.size > 1)
                item(key = "server/$server") {
                    Text(
                        server,
                        fontSize = 13.sp,
                        fontWeight = FontWeight.SemiBold,
                        color = Palette.muted,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.padding(start = Dims.gutter, end = Dims.gutter, top = 12.dp, bottom = 4.dp),
                    )
                }
            items(libs, key = { it.server + "/" + it.id }) { lib ->
                Row(
                    Modifier.fillMaxWidth()
                        .clickable { onOpen(lib) }
                        .heightIn(min = 56.dp)
                        .padding(horizontal = Dims.gutter),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.spacedBy(16.dp),
                ) {
                    GlyphIcon(if (lib.movies) Glyph.Movie else Glyph.Tv, 22.dp, Palette.muted)
                    Text(
                        lib.title,
                        fontSize = 16.sp,
                        color = Palette.text,
                        maxLines = 1,
                        overflow = TextOverflow.Ellipsis,
                        modifier = Modifier.weight(1f),
                    )
                    GlyphIcon(Glyph.ChevronRight, 20.dp, Palette.muted)
                }
            }
        }
    }
}
