package xyz.spikers.plaivo

import android.content.Intent
import android.os.Bundle
import androidx.activity.compose.BackHandler
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.Canvas
import androidx.compose.animation.core.animateDpAsState
import androidx.compose.animation.core.tween
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.MaterialTheme
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.withContext
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.Section
import uniffi.plaivo_ffi.PlaivoException
import uniffi.plaivo_ffi.SignIn as Pin
import kotlin.math.floor

class MainActivity : ComponentActivity() {
    private var resumes by mutableIntStateOf(0)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            MaterialTheme(colorScheme = plaivoColors) {
                Surface(Modifier.fillMaxSize()) {
                    var signedIn by remember { mutableStateOf(core.signedIn()) }
                    if (signedIn) Shell(resumes, ::play) { signedIn = false } else SignInScreen { signedIn = true }
                }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        resumes++
    }

    private fun play(card: MediaCard) {
        startActivity(Intent(this, PlayerActivity::class.java).putExtra("server", card.server).putExtra("key", card.key))
    }
}

private const val POLL_MS = 2000L
private const val RETRY_MS = 3000L

@Composable
private fun SignInScreen(onDone: () -> Unit) {
    var pin by remember { mutableStateOf<Pin?>(null) }
    var problem by remember { mutableStateOf<String?>(null) }
    LaunchedEffect(Unit) {
        while (true) {
            val next = try {
                withContext(Dispatchers.IO) { core.startSignIn() }
            } catch (e: PlaivoException) {
                problem = e.message
                delay(RETRY_MS)
                continue
            }
            pin = next
            problem = null
            val expires = System.currentTimeMillis() + next.expiresInS.toLong() * 1000
            while (System.currentTimeMillis() < expires) {
                delay(POLL_MS)
                val linked = try {
                    withContext(Dispatchers.IO) { core.finishSignIn(next.id) }.also { problem = null }
                } catch (e: PlaivoException) {
                    problem = e.message
                    false
                }
                if (linked) {
                    onDone()
                    return@LaunchedEffect
                }
            }
        }
    }
    val colors = MaterialTheme.colorScheme
    Column(Modifier.fillMaxSize(), verticalArrangement = Arrangement.Center, horizontalAlignment = Alignment.CenterHorizontally) {
        Text("Sign in to Plex", style = MaterialTheme.typography.headlineMedium)
        Spacer(Modifier.height(48.dp))
        Row(verticalAlignment = Alignment.CenterVertically) {
            Column(Modifier.width(360.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text("On your phone or computer, go to", style = MaterialTheme.typography.titleMedium, color = colors.onSurfaceVariant)
                Text("plex.tv/link", style = MaterialTheme.typography.headlineSmall, color = colors.primary)
                Spacer(Modifier.height(24.dp))
                Text("and enter this code", style = MaterialTheme.typography.titleMedium, color = colors.onSurfaceVariant)
                Spacer(Modifier.height(8.dp))
                Text(pin?.code ?: "····", style = MaterialTheme.typography.displayLarge, letterSpacing = 12.sp)
            }
            Column(Modifier.padding(horizontal = 40.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Box(Modifier.width(1.dp).height(88.dp).background(colors.border))
                Text("OR", style = MaterialTheme.typography.labelLarge, color = colors.onSurfaceVariant, modifier = Modifier.padding(vertical = 12.dp))
                Box(Modifier.width(1.dp).height(88.dp).background(colors.border))
            }
            Column(Modifier.width(360.dp), horizontalAlignment = Alignment.CenterHorizontally) {
                Text("Scan with your phone's camera", style = MaterialTheme.typography.titleMedium, color = colors.onSurfaceVariant)
                Spacer(Modifier.height(16.dp))
                val qr = pin
                if (qr != null) QrCode(qr.qrSize.toInt(), qr.qr, Modifier.size(220.dp))
                else Box(Modifier.size(220.dp).background(colors.surfaceVariant, RoundedCornerShape(12.dp)))
            }
        }
        Text(
            problem?.let { "Can't reach Plex, retrying… ($it)" } ?: "",
            style = MaterialTheme.typography.bodyMedium,
            color = colors.onSurfaceVariant,
            modifier = Modifier.padding(top = 40.dp),
        )
    }
}

@Composable
private fun QrCode(modules: Int, dark: List<Boolean>, modifier: Modifier) {
    Canvas(modifier.background(Color.White, RoundedCornerShape(12.dp))) {
        val quiet = 4
        val cell = floor(size.minDimension / (modules + quiet * 2))
        val origin = (size.minDimension - cell * modules) / 2
        for (y in 0 until modules) for (x in 0 until modules) {
            if (dark[y * modules + x]) drawRect(Color.Black, Offset(origin + x * cell, origin + y * cell), Size(cell, cell))
        }
    }
}

@Composable
private fun Shell(refresh: Int, play: (MediaCard) -> Unit, onSignedOut: () -> Unit) {
    val prefs = App.instance.prefs
    var pinned by remember { mutableStateOf(!prefs.getBoolean(PREF_SIDEBAR_COMPACT, true)) }
    var homeId by remember { mutableStateOf(prefs.getString(PREF_HOME_SERVER, null)) }
    var screen by remember { mutableStateOf<Screen>(Screen.Home) }
    var servers by remember { mutableStateOf(core.servers()) }
    var serversLoaded by remember { mutableStateOf(false) }
    var libraries by remember { mutableStateOf<List<Section>>(emptyList()) }
    var inSidebar by remember { mutableStateOf(false) }
    var focusHome by remember { mutableStateOf(true) }
    val sidebarFocus = remember { FocusRequester() }
    LaunchedEffect(refresh) {
        if (libraries.isEmpty()) libraries = io { core.libraries(true) }
        io { runCatching { core.refreshServers() } }.onSuccess { servers = it }
        serversLoaded = true
        libraries = io { core.libraries(false) }
    }
    val home = servers.firstOrNull { it.id == homeId } ?: servers.firstOrNull()
    BackHandler(enabled = !inSidebar || screen != Screen.Home) {
        if (!inSidebar) runCatching { sidebarFocus.requestFocus() } else screen = Screen.Home
    }
    val inset by animateDpAsState(if (pinned) Dims.sidebar else Dims.sidebarCompact, tween(180, easing = EaseOutQuart), label = "inset")
    Box(Modifier.fillMaxSize().background(Palette.bg)) {
        Box(Modifier.fillMaxSize().padding(start = inset)) {
            when (val s = screen) {
                Screen.Home -> HomeScreen(home, serversLoaded, refresh, focusHome, { focusHome = false }, play)
                Screen.Search -> SearchScreen(play)
                Screen.Settings -> SettingsScreen(servers, home?.id, onHome = {
                    homeId = it
                    prefs.edit().putString(PREF_HOME_SERVER, it).apply()
                }, onSignOut = {
                    core.signOut()
                    prefs.edit().remove(PREF_HOME_SERVER).apply()
                    onSignedOut()
                })
                is Screen.Browse -> LibraryScreen(s.library, play)
            }
        }
        Sidebar(
            pinned = pinned,
            screen = screen,
            libraries = libraries.sortedBy { it.server != home?.id },
            focus = sidebarFocus,
            onTogglePin = {
                pinned = !pinned
                prefs.edit().putBoolean(PREF_SIDEBAR_COMPACT, !pinned).apply()
            },
            onNavigate = { screen = it },
            onFocusChanged = { inSidebar = it },
        )
    }
}
