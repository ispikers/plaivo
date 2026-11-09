package xyz.spikers.plaivo

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.aspectRatio
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.staticCompositionLocalOf
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.layout.ContentScale
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Border
import androidx.tv.material3.Card
import androidx.tv.material3.CardDefaults
import androidx.tv.material3.Text
import coil3.compose.AsyncImage
import uniffi.plaivo_ffi.Card as MediaCard

private val cardShape = RoundedCornerShape(8.dp)

val LocalCardMenu = staticCompositionLocalOf<((MediaCard) -> Unit)?> { null }

val LocalPlay = staticCompositionLocalOf<(MediaCard) -> Unit> { {} }

@Composable
fun MediaTile(card: MediaCard, wide: Boolean, modifier: Modifier = Modifier, dim: Boolean = false, onFocus: () -> Unit = {}, onClick: () -> Unit) {
    val menu = LocalCardMenu.current
    Column(modifier.width((if (wide) Dims.wide else Dims.poster) * LocalCardScale.current)) {
        Card(
            onClick = onClick,
            onLongClick = menu?.let { { it(card) } },
            modifier = Modifier.fillMaxWidth().aspectRatio(if (wide) 16f / 9f else 2f / 3f).onFocusChanged { if (it.isFocused) onFocus() },
            shape = CardDefaults.shape(cardShape),
            colors = CardDefaults.colors(containerColor = Palette.surface, focusedContainerColor = Palette.surface),
            scale = CardDefaults.scale(focusedScale = 1f),
            border = CardDefaults.border(focusedBorder = Border(BorderStroke(2.5.dp, Palette.text), shape = cardShape)),
            glow = CardDefaults.glow(),
        ) {
            Box(Modifier.fillMaxSize()) {
                AsyncImage(model = card.image, contentDescription = card.title, contentScale = ContentScale.Crop, modifier = Modifier.fillMaxSize())
                if (dim) Box(Modifier.fillMaxSize().background(Color.Black.copy(alpha = 0.3f)))
                if (card.progress > 0f && !card.watched) {
                    Box(Modifier.align(Alignment.BottomCenter).padding(6.dp).fillMaxWidth().height(4.dp).background(Palette.surface, CircleShape)) {
                        Box(Modifier.fillMaxWidth(card.progress).height(4.dp).background(Palette.text, CircleShape))
                    }
                }
                if (card.watched && LocalAppearance.current.watchedIndicators) {
                    Box(Modifier.align(Alignment.TopEnd).padding(4.dp).size(22.dp).shadow(4.dp, CircleShape).background(Palette.text, CircleShape), contentAlignment = Alignment.Center) {
                        GlyphIcon(Glyph.Check, 15.dp, Palette.bg)
                    }
                } else if (!card.watched && card.unwatched > 0uL) {
                    Box(
                        Modifier.align(Alignment.TopEnd).padding(4.dp).height(22.dp).widthIn(min = 22.dp).shadow(4.dp, CircleShape).background(Palette.text, CircleShape).padding(horizontal = 6.dp),
                        contentAlignment = Alignment.Center,
                    ) {
                        Text(if (card.unwatched > 999uL) "999+" else card.unwatched.toString(), fontSize = 11.sp, fontWeight = FontWeight.Bold, color = Palette.bg)
                    }
                }
            }
        }
        Text(card.title, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 12.sp, fontWeight = FontWeight.SemiBold, color = Palette.text, modifier = Modifier.padding(top = 6.dp))
        Text(card.subtitle, maxLines = 1, overflow = TextOverflow.Ellipsis, fontSize = 11.sp, color = Palette.muted)
    }
}

@Composable
fun SeeAll(wide: Boolean, dim: Boolean, onClick: () -> Unit) {
    Column(Modifier.width((if (wide) Dims.wide else Dims.poster) * LocalCardScale.current)) {
        Card(
            onClick = onClick,
            modifier = Modifier.fillMaxWidth().aspectRatio(if (wide) 16f / 9f else 2f / 3f),
            shape = CardDefaults.shape(cardShape),
            colors = CardDefaults.colors(containerColor = Palette.surface, focusedContainerColor = Palette.tint(0.12f)),
            scale = CardDefaults.scale(focusedScale = 1f),
            border = CardDefaults.border(focusedBorder = Border(BorderStroke(2.5.dp, Palette.text), shape = cardShape)),
            glow = CardDefaults.glow(),
        ) {
            Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.spacedBy(6.dp, Alignment.CenterVertically), horizontalAlignment = Alignment.CenterHorizontally) {
                val tint = Palette.text.copy(alpha = if (dim) 0.54f else 1f)
                GlyphIcon(Glyph.ChevronRight, 28.dp, tint)
                Text("See all", fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = tint)
            }
        }
    }
}
