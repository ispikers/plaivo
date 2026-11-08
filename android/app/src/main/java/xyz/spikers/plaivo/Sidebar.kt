package xyz.spikers.plaivo

import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.animateFloatAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.focusGroup
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.alpha
import androidx.compose.ui.draw.drawBehind
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusProperties
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import uniffi.plaivo_ffi.Section

sealed interface Screen {
    data object Home : Screen
    data object Search : Screen
    data object Downloads : Screen
    data object Settings : Screen
    data object AddServer : Screen
    data class Browse(val library: Section) : Screen
}

private const val SIDEBAR_ANIM_MS = 180

@Composable
fun Sidebar(
    pinned: Boolean,
    screen: Screen,
    libraries: List<Section>,
    focus: FocusRequester,
    onTogglePin: () -> Unit,
    onNavigate: (Screen) -> Unit,
    onFocusChanged: (Boolean) -> Unit,
    modifier: Modifier = Modifier,
) {
    var focused by remember { mutableStateOf(false) }
    val expanded = pinned || focused
    val width by animateDpAsState(if (expanded) Dims.sidebar else Dims.sidebarCompact, tween(SIDEBAR_ANIM_MS, easing = EaseOutQuart), label = "sidebar")
    val labelAlpha by animateFloatAsState(if (expanded) 1f else 0f, tween(SIDEBAR_ANIM_MS, easing = EaseOutQuart), label = "labels")
    val groups = libraries.groupBy { it.serverName }
    fun Modifier.selectedFocus(selected: Boolean) = if (selected) focusRequester(focus) else this
    Column(
        modifier
            .width(width)
            .fillMaxHeight()
            .shadow(if (expanded && !pinned) 24.dp else 0.dp)
            .background(Palette.bg)
            .drawBehind { drawLine(Palette.outline, Offset(size.width - 0.5f, 0f), Offset(size.width - 0.5f, size.height), 1f) }
            .padding(start = 10.dp, end = 10.dp, top = 14.dp, bottom = 10.dp)
            .onFocusChanged {
                focused = it.hasFocus
                onFocusChanged(it.hasFocus)
            }
            .focusProperties { onEnter = { runCatching { focus.requestFocus() } } }
            .focusGroup(),
        verticalArrangement = Arrangement.spacedBy(2.dp),
    ) {
        NavItem(Glyph.Menu, "Plaivo", pinned, labelAlpha, onTogglePin)
        Spacer(Modifier.height(8.dp))
        NavItem(Glyph.Home, "Home", screen == Screen.Home, labelAlpha, { onNavigate(Screen.Home) }, Modifier.selectedFocus(screen == Screen.Home))
        NavItem(Glyph.Search, "Search", screen == Screen.Search, labelAlpha, { onNavigate(Screen.Search) }, Modifier.selectedFocus(screen == Screen.Search))
        NavItem(Glyph.Download, "Downloads", screen == Screen.Downloads, labelAlpha, { onNavigate(Screen.Downloads) }, Modifier.selectedFocus(screen == Screen.Downloads))
        Box(Modifier.fillMaxWidth().height(32.dp)) {
            Text("LIBRARIES", fontSize = 10.sp, fontWeight = FontWeight.SemiBold, color = Palette.muted, softWrap = false, modifier = Modifier.align(Alignment.BottomStart).padding(start = 11.dp, bottom = 5.dp).alpha(labelAlpha))
            Box(Modifier.align(Alignment.BottomStart).padding(start = 11.dp, bottom = 10.dp).width(20.dp).height(1.dp).alpha(1f - labelAlpha).background(Palette.outline))
        }
        Column(Modifier.weight(1f).fillMaxWidth().verticalScroll(rememberScrollState()), verticalArrangement = Arrangement.spacedBy(2.dp)) {
            groups.forEach { (server, libs) ->
                if (groups.size > 1) {
                    Box(Modifier.fillMaxWidth().height(24.dp).padding(start = 12.dp, bottom = 3.dp), contentAlignment = Alignment.BottomStart) {
                        Text(server, fontSize = 11.sp, fontWeight = FontWeight.SemiBold, color = Palette.muted, maxLines = 1, softWrap = false, modifier = Modifier.alpha(labelAlpha))
                    }
                }
                libs.forEach { lib ->
                    val selected = screen == Screen.Browse(lib)
                    NavItem(if (lib.movies) Glyph.Movie else Glyph.Tv, lib.title, selected, labelAlpha, { onNavigate(Screen.Browse(lib)) }, Modifier.selectedFocus(selected))
                }
            }
        }
        NavItem(Glyph.Add, "Add Library", screen == Screen.AddServer, labelAlpha, { onNavigate(Screen.AddServer) }, Modifier.selectedFocus(screen == Screen.AddServer))
        NavItem(Glyph.Settings, "Settings", screen == Screen.Settings, labelAlpha, { onNavigate(Screen.Settings) }, Modifier.selectedFocus(screen == Screen.Settings))
    }
}

@Composable
private fun NavItem(glyph: Glyph, label: String, selected: Boolean, labelAlpha: Float, onClick: () -> Unit, modifier: Modifier = Modifier) {
    Surface(
        onClick = onClick,
        modifier = modifier.fillMaxWidth().height(36.dp),
        shape = ClickableSurfaceDefaults.shape(RoundedCornerShape(9.dp)),
        colors = ClickableSurfaceDefaults.colors(
            containerColor = if (selected) Palette.tint(0.10f) else Color.Transparent,
            contentColor = Palette.text,
            focusedContainerColor = Palette.tint(if (selected) 0.15f else 0.12f),
            focusedContentColor = Palette.text,
            pressedContainerColor = Palette.tint(0.15f),
            pressedContentColor = Palette.text,
        ),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
    ) {
        Row(Modifier.fillMaxSize().padding(start = 12.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(10.dp)) {
            GlyphIcon(glyph, 18.dp, if (selected) Palette.text else Palette.muted)
            Text(
                label,
                modifier = Modifier.width(LABEL_WIDTH).alpha(labelAlpha),
                fontSize = 13.sp,
                fontWeight = if (selected) FontWeight.SemiBold else FontWeight.Normal,
                color = if (selected) Palette.text else Palette.muted,
                maxLines = 1,
                softWrap = false,
                overflow = TextOverflow.Ellipsis,
            )
        }
    }
}

private val LABEL_WIDTH: Dp = 132.dp
