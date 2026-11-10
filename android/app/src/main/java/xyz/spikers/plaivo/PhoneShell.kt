package xyz.spikers.plaivo

import android.content.Intent
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.consumeWindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.imePadding
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.Button
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.NavigationBar
import androidx.compose.material3.NavigationBarItem
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.layout.onSizeChanged
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.net.toUri
import androidx.tv.material3.MaterialTheme
import uniffi.plaivo_ffi.SignIn as Pin

enum class PhoneTab(val label: String, val glyph: Glyph) {
    Home("Home", Glyph.Home),
    Libraries("Libraries", Glyph.Layers),
    Watchlist("Watchlist", Glyph.Bookmark),
    Downloads("Downloads", Glyph.Download),
    Settings("Settings", Glyph.Settings),
}

@Composable
fun PhoneShell(
    tab: PhoneTab,
    tabs: List<PhoneTab>,
    onTab: (PhoneTab) -> Unit,
    content: @Composable (PhoneTab) -> Unit,
) {
    val density = LocalDensity.current
    var bar by remember { mutableStateOf(0.dp) }
    Column(Modifier.fillMaxSize().background(Palette.bg)) {
        val sides =
            if (tab == PhoneTab.Home) WindowInsetsSides.Horizontal
            else WindowInsetsSides.Horizontal + WindowInsetsSides.Top
        Box(
            Modifier.weight(1f)
                .fillMaxWidth()
                .windowInsetsPadding(WindowInsets.safeDrawing.only(sides))
                .consumeWindowInsets(PaddingValues(bottom = bar))
                .imePadding()
        ) {
            content(tab)
        }
        NavigationBar(Modifier.onSizeChanged { bar = with(density) { it.height.toDp() } }) {
            tabs.forEach { t ->
                NavigationBarItem(
                    selected = t == tab,
                    onClick = { onTab(t) },
                    icon = { GlyphIcon(t.glyph, 24.dp, LocalContentColor.current) },
                    label = { Text(t.label, fontSize = 11.sp, maxLines = 1, softWrap = false) },
                )
            }
        }
    }
}

@Composable
fun TouchSignIn(pin: Pin?, problem: String?, onJellyfin: () -> Unit) {
    val context = LocalContext.current
    Column(
        Modifier.fillMaxSize().safeDrawingPadding().verticalScroll(rememberScrollState()).padding(Dims.gutter),
        verticalArrangement = Arrangement.spacedBy(16.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Text("Sign in to Plex", style = MaterialTheme.typography.headlineMedium, color = Palette.text)
        Button(
            onClick = {
                pin?.let { runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, it.url.toUri())) } }
            },
            enabled = pin != null,
            modifier = Modifier.fillMaxWidth().height(52.dp),
        ) {
            Text("Sign in with browser", fontSize = 16.sp)
        }
        Spacer(Modifier.height(8.dp))
        Text(
            "Or go to plex.tv/link on another device and enter this code",
            color = Palette.muted,
            textAlign = TextAlign.Center,
        )
        Text(
            pin?.code ?: "····",
            style = MaterialTheme.typography.displayMedium,
            color = Palette.text,
            letterSpacing = 10.sp,
        )
        Text("or scan with another phone's camera", color = Palette.muted)
        if (pin != null) QrCode(pin.qrSize.toInt(), pin.qr, Modifier.size(200.dp))
        else Box(Modifier.size(200.dp).background(Palette.surface, RoundedCornerShape(12.dp)))
        if (problem != null) Text("Can't reach Plex, retrying… ($problem)", color = Palette.muted)
        TextButton(onClick = onJellyfin) { Text("Add a Jellyfin server instead") }
    }
}
