package xyz.spikers.plaivo

import android.content.Intent
import android.os.Build
import android.os.Handler
import android.os.Looper
import android.widget.Toast
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.key
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Text
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.WatchCommand
import uniffi.plaivo_ffi.WatchConnection
import uniffi.plaivo_ffi.WatchListener
import uniffi.plaivo_ffi.WatchState
import uniffi.plaivo_ffi.formatRoomCode
import uniffi.plaivo_ffi.rememberRoom

object Watch : WatchListener {
    data class Bound(val code: String, val epoch: Long, val server: String, val key: String)

    private val main = Handler(Looper.getMainLooper())
    var state by mutableStateOf<WatchState?>(null)
        private set

    var item by mutableStateOf<Bound?>(null)
        private set

    var rooms by mutableStateOf<List<String>>(emptyList())
        private set

    var player: PlayerActivity? = null
    private var pendingShare: MediaCard? = null
    private var resolving: Pair<String, Long>? = null

    val inRoom
        get() = state?.connection.let { it == WatchConnection.CONNECTED || it == WatchConnection.RECONNECTING }

    fun start() {
        rooms = core.settings().watchRooms
        core.watchListen(this)
    }

    override fun state(state: WatchState) {
        main.post { onState(state) }
    }

    override fun notice(text: String) {
        main.post { toast(text) }
    }

    override fun command(command: WatchCommand) {
        main.post { player?.follow(command) }
    }

    fun deviceName(): String =
        android.provider.Settings.Global.getString(App.instance.contentResolver, "device_name") ?: Build.MODEL

    fun create() =
        background.launch {
            runCatching { core.watchCreate(deviceName()) }
                .onFailure {
                    main.post {
                        pendingShare = null
                        toast(it.message)
                    }
                }
        }

    fun join(code: String) =
        background.launch {
            runCatching { core.watchJoin(code, deviceName()) }.onFailure { main.post { toast(it.message) } }
        }

    fun share(card: MediaCard) {
        if (!inRoom) {
            pendingShare = card
            create()
            return
        }
        background.launch {
            runCatching { core.watchShare(card.server, card.key) }.onFailure { main.post { toast(it.message) } }
        }
    }

    fun epochFor(server: String, key: String): Long? =
        item?.takeIf { inRoom && it.server == server && it.key == key }?.epoch

    fun loaded(server: String, key: String, durationMs: Long) {
        epochFor(server, key)?.let { core.watchMatched(it, durationMs) }
    }

    fun failed(server: String, key: String) {
        epochFor(server, key)?.let { core.watchUnavailable(it) }
    }

    fun open() {
        val b = item ?: return
        val p = player
        if (p != null) return if (p.started()) p.watch(b) else Unit
        App.instance.startActivity(
            Intent(App.instance, PlayerActivity::class.java)
                .addFlags(Intent.FLAG_ACTIVITY_NEW_TASK)
                .putExtra("server", b.server)
                .putExtra("key", b.key)
        )
    }

    private fun onState(s: WatchState) {
        val before = state
        state = s
        if (!inRoom) {
            if (s.connection != WatchConnection.CONNECTING) pendingShare = null
            item = null
            resolving = null
            return
        }
        if (s.code != before?.code) rooms = editSettings { rememberRoom(it, s.code) }.watchRooms
        if (s.host)
            pendingShare?.let {
                pendingShare = null
                share(it)
            }
        val target = s.code to s.epoch
        if (s.media == null) {
            item = null
            resolving = target
            return
        }
        if (target == resolving) return
        resolving = target
        item = null
        background.launch {
            val found = runCatching { core.watchResolve(s.epoch) }.getOrNull()
            main.post {
                if (resolving != target) return@post
                if (found == null) {
                    core.watchUnavailable(s.epoch)
                    toast("${s.media} isn't on your servers")
                } else {
                    item = Bound(s.code, s.epoch, found.server, found.key)
                    open()
                }
            }
        }
    }

    private fun toast(text: String?) {
        if (!text.isNullOrEmpty()) Toast.makeText(App.instance, text, Toast.LENGTH_SHORT).show()
    }
}

@Composable
fun WatchTogetherScreen() {
    val s = Watch.state
    val connection = s?.connection ?: WatchConnection.IDLE
    val anchor = remember { FocusRequester() }
    var focused by remember { mutableStateOf(false) }
    val layout = listOf(connection, s?.host, s?.media, s?.peers?.map { it.peerId })
    val hadFocus = remember(layout) { focused }
    LaunchedEffect(layout) {
        if (!hadFocus) return@LaunchedEffect
        delay(50)
        if (!focused) runCatching { anchor.requestFocus() }
    }
    var code by rememberSaveable { mutableStateOf("") }
    fun join() {
        if (code.isNotBlank()) Watch.join(code)
    }
    Column(
        Modifier.fillMaxSize()
            .onFocusChanged { focused = it.hasFocus }
            .verticalScroll(rememberScrollState())
            .padding(PaddingValues(horizontal = Dims.edge, vertical = 28.dp)),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Heading("Watch Together")
        when {
            s != null && Watch.inRoom -> {
                Text("Room code", fontSize = 13.sp, color = Palette.muted)
                Text(
                    s.displayCode,
                    fontSize = 64.sp,
                    fontWeight = FontWeight.Bold,
                    color = Palette.text,
                    letterSpacing = 6.sp,
                )
                Note(
                    if (connection == WatchConnection.RECONNECTING) "Reconnecting…"
                    else "Friends join with this code in Plaivo or Plezy."
                )
                SectionTitle("Watching")
                val media = s.media
                if (media == null) {
                    Note(
                        if (s.host) "Hold OK on any poster and choose Watch together."
                        else "Waiting for the host to pick something."
                    )
                } else {
                    Row(
                        horizontalArrangement = Arrangement.spacedBy(8.dp),
                        verticalAlignment = Alignment.CenterVertically,
                    ) {
                        SettingsRow(Glyph.PlayCircle, media, s.status, Modifier.weight(1f), onClick = Watch::open)
                        if (Watch.item != null) ActionButton("Open party", Watch::open)
                    }
                    if (s.host)
                        SettingsRow(
                            Glyph.Stop,
                            "Stop sharing",
                            "Clears what the room is watching",
                            onClick = core::watchClearMedia,
                        )
                }
                SectionTitle("People")
                s.peers.forEach { p ->
                    key(p.peerId) {
                        Row(
                            horizontalArrangement = Arrangement.spacedBy(8.dp),
                            verticalAlignment = Alignment.CenterVertically,
                        ) {
                            SettingsRow(
                                Glyph.Person,
                                if (p.me) "${p.name} (you)" else p.name,
                                p.status,
                                Modifier.weight(1f),
                            ) {
                                if (p.host)
                                    Text(
                                        "Host",
                                        fontSize = 12.sp,
                                        fontWeight = FontWeight.SemiBold,
                                        color = Palette.bg,
                                        modifier =
                                            Modifier.background(Palette.text, CircleShape)
                                                .padding(horizontal = 10.dp, vertical = 3.dp),
                                    )
                            }
                            if (s.host && !p.me) {
                                ActionButton("Make host", { core.watchTransferHost(p.peerId) })
                                ActionButton("Remove", { core.watchKick(p.peerId) })
                            }
                        }
                    }
                }
                if (s.host) {
                    SectionTitle("Room")
                    SettingsRow(
                        Glyph.PlayCircle,
                        "Who controls playback",
                        "Play, pause, seek and speed",
                        onClick = { core.watchSetControl(!s.everyoneControls) },
                    ) {
                        Value(if (s.everyoneControls) "Everyone" else "Host only")
                    }
                    SettingsRow(
                        Glyph.Lock,
                        "Lock room",
                        "New people can't join",
                        onClick = { core.watchLock(!s.locked) },
                    ) {
                        Switch(s.locked)
                    }
                    SettingsRow(Glyph.Close, "End room", "Closes the room for everyone", onClick = core::watchEnd)
                }
                SettingsRow(Glyph.Logout, "Leave room", "", Modifier.focusRequester(anchor), onClick = core::watchLeave)
            }
            connection == WatchConnection.CONNECTING -> {
                Note("Connecting…")
                ActionButton("Cancel", core::watchLeave, Modifier.focusRequester(anchor))
            }
            else -> {
                Note("Watch in sync with friends. Rooms work with Plaivo and Plezy.")
                SettingsRow(
                    Glyph.Add,
                    "Create a room",
                    "You pick what everyone watches",
                    Modifier.focusRequester(anchor),
                    onClick = { Watch.create() },
                )
                SectionTitle("Join a room")
                Row(
                    horizontalArrangement = Arrangement.spacedBy(10.dp),
                    verticalAlignment = Alignment.CenterVertically,
                ) {
                    Field(
                        code,
                        { code = it.uppercase().take(12) },
                        "Room code, like ABC-234",
                        KeyboardType.Ascii,
                        ImeAction.Go,
                        ::join,
                    )
                    ActionButton("Join", ::join)
                }
                if (Watch.rooms.isNotEmpty()) {
                    SectionTitle("Recent rooms")
                    Watch.rooms.forEach { c ->
                        SettingsRow(Glyph.Group, formatRoomCode(c), "", onClick = { Watch.join(c) })
                    }
                }
            }
        }
    }
}

@Composable
fun WatchPill(s: WatchState, modifier: Modifier = Modifier) {
    val status = if (s.connection == WatchConnection.RECONNECTING) "Reconnecting" else s.status
    Row(
        modifier.background(Color.Black.copy(alpha = 0.6f), CircleShape).padding(horizontal = 12.dp, vertical = 6.dp),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.spacedBy(8.dp),
    ) {
        GlyphIcon(Glyph.Group, 16.dp, Color.White)
        Text(
            listOf(s.displayCode, "${s.peers.size} watching", status).filter { it.isNotEmpty() }.joinToString(" · "),
            fontSize = 13.sp,
            color = Color.White,
        )
    }
}
