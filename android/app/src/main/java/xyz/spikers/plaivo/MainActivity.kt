package xyz.spikers.plaivo

import android.content.Intent
import android.os.Bundle
import android.os.SystemClock
import android.widget.Toast
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
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableLongStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.MaterialTheme
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.Section
import uniffi.plaivo_ffi.PlaivoException
import uniffi.plaivo_ffi.SignIn as Pin
import uniffi.plaivo_ffi.cardScale
import kotlin.math.floor

class MainActivity : ComponentActivity() {
    private var resumes by mutableIntStateOf(0)

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        setContent {
            PlaivoTheme {
                Surface(Modifier.fillMaxSize()) {
                    var signedIn by remember { mutableStateOf(core.hasServers()) }
                    var session by remember { mutableIntStateOf(0) }
                    var addingFirst by remember { mutableStateOf(false) }
                    val enter: () -> Unit = {
                        addingFirst = false
                        signedIn = true
                        session++
                    }
                    when {
                        signedIn -> key(session) {
                            Shell(resumes, ::play, onAccountChanged = {
                                signedIn = core.hasServers()
                                session++
                            }, onPlexSignIn = { signedIn = false })
                        }
                        addingFirst -> AddServerScreen(onBack = { addingFirst = false }) { enter() }
                        else -> SignInScreen(onJellyfin = { addingFirst = true }, onBack = if (core.hasServers()) enter else null, onDone = enter)
                    }
                }
            }
        }
    }

    override fun onResume() {
        super.onResume()
        DownloadService.unblock()
        resumes++
    }

    private fun play(card: MediaCard) {
        startActivity(Intent(this, PlayerActivity::class.java).putExtra("server", card.server).putExtra("key", card.key))
    }
}

private const val POLL_MS = 2000L
private const val RETRY_MS = 3000L
private const val REDISCOVER_MS = 10 * 60 * 1000L

@Composable
private fun SignInScreen(onJellyfin: () -> Unit, onBack: (() -> Unit)?, onDone: () -> Unit) {
    if (onBack != null) BackHandler(onBack = onBack)
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
        Spacer(Modifier.height(16.dp))
        ActionButton("Add a Jellyfin server instead", onJellyfin)
    }
}

@Composable
fun QrCode(modules: Int, dark: List<Boolean>, modifier: Modifier) {
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
private fun Shell(refresh: Int, play: (MediaCard) -> Unit, onAccountChanged: () -> Unit, onPlexSignIn: () -> Unit) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope()
    var settings by remember(refresh) { mutableStateOf(core.settings()) }
    val pinned = !settings.sidebarCompact
    var screen by remember { mutableStateOf<Screen>(Screen.Home) }
    var servers by remember { mutableStateOf(core.servers()) }
    var serversLoaded by remember { mutableStateOf(false) }
    var libraries by remember { mutableStateOf<List<Section>>(emptyList()) }
    var inSidebar by remember { mutableStateOf(false) }
    var focusHome by remember { mutableStateOf(true) }
    val sidebarFocus = remember { FocusRequester() }
    var menu by remember { mutableStateOf<MediaCard?>(null) }
    var changes by remember { mutableIntStateOf(0) }
    val openMenu = remember { { card: MediaCard -> menu = card } }
    fun reloadServers() {
        if (!core.hasServers()) return onAccountChanged()
        servers = core.servers()
        val ids = servers.map { it.id }.toSet()
        libraries = libraries.filter { it.server in ids }
        (screen as? Screen.Browse)?.let { if (it.library.server !in ids) screen = Screen.Home }
        scope.launch { libraries = io { core.libraries(false) } }
    }
    var discoveredAt by remember { mutableLongStateOf(0L) }
    LaunchedEffect(refresh) {
        if (libraries.isEmpty()) libraries = io { core.libraries(true) }
        if (serversLoaded && SystemClock.elapsedRealtime() - discoveredAt < REDISCOVER_MS) return@LaunchedEffect
        io { runCatching { core.refreshServers() } }.onSuccess {
            servers = it
            discoveredAt = SystemClock.elapsedRealtime()
        }
        serversLoaded = true
        libraries = io { core.libraries(false) }
    }
    val home = servers.firstOrNull { it.id == settings.homeServer } ?: servers.firstOrNull()
    val arranged = remember(libraries, settings.libraryOrder) { core.arrangeLibraries(libraries) }
    val shown = remember(arranged, settings.hiddenLibraries, home) { arranged.filter { it.id !in settings.hiddenLibraries }.sortedBy { it.server != home?.id } }
    val cardScale = remember(settings.density) { cardScale(settings.density) }
    val appearance = remember(settings) { appearanceOf(settings) }
    BackHandler(enabled = !inSidebar || screen != Screen.Home) {
        if (!inSidebar) runCatching { sidebarFocus.requestFocus() } else screen = Screen.Home
    }
    val inset by animateDpAsState(if (pinned) Dims.sidebar else Dims.sidebarCompact, tween(180, easing = EaseOutQuart), label = "inset")
    Box(Modifier.fillMaxSize().background(Palette.bg)) {
        Box(Modifier.fillMaxSize().padding(start = inset)) {
            CompositionLocalProvider(LocalCardScale provides cardScale, LocalCardMenu provides openMenu, LocalAppearance provides appearance) {
                when (val s = screen) {
                    Screen.Home -> HomeScreen(home, serversLoaded, refresh + changes, focusHome, { focusHome = false }, play) { screen = Screen.Downloads }
                    Screen.Search -> SearchScreen(play)
                    Screen.Downloads -> DownloadsScreen(play)
                    Screen.WatchTogether -> WatchTogetherScreen()
                    Screen.Settings -> SettingsScreen(
                        servers,
                        home,
                        settings,
                        arranged,
                        onEdit = { change -> settings = editSettings(change) },
                        onSignOut = {
                            scope.launch {
                                io { core.signOut() }
                                onAccountChanged()
                            }
                        },
                        onPlexSignIn = onPlexSignIn,
                        onServersChanged = ::reloadServers,
                    )
                    Screen.AddServer -> AddServerScreen(onBack = null) { name ->
                        Toast.makeText(context, "Added $name", Toast.LENGTH_SHORT).show()
                        reloadServers()
                        focusHome = true
                        screen = Screen.Home
                    }
                    is Screen.Browse -> LibraryScreen(s.library, play)
                }
            }
        }
        Sidebar(
            pinned = pinned,
            screen = screen,
            libraries = shown,
            focus = sidebarFocus,
            onTogglePin = { settings = editSettings { it.copy(sidebarCompact = !it.sidebarCompact) } },
            onNavigate = { screen = it },
            onFocusChanged = { inSidebar = it },
        )
    }
    menu?.let { CardMenu(it, onDismiss = { menu = null }, onChanged = { changes++ }) }
}
