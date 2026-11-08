package xyz.spikers.plaivo

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.text.BasicTextField
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.focus.onFocusChanged
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.text.TextStyle
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.text.input.KeyboardType
import androidx.compose.ui.text.input.PasswordVisualTransformation
import androidx.compose.ui.text.input.VisualTransformation
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.tv.material3.Border
import androidx.tv.material3.ClickableSurfaceDefaults
import androidx.tv.material3.Surface
import androidx.tv.material3.Text
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.plaivo_ffi.JellyfinProbe
import uniffi.plaivo_ffi.PlaivoException

private const val QUICK_CONNECT_POLL_MS = 2000L
private val ERROR = Color(0xFFFF8A80)

@Composable
fun AddServerScreen(onBack: (() -> Unit)?, onAdded: (String) -> Unit) {
    val scope = rememberCoroutineScope()
    var address by rememberSaveable { mutableStateOf("") }
    var username by rememberSaveable { mutableStateOf("") }
    var password by remember { mutableStateOf("") }
    var found by remember { mutableStateOf<JellyfinProbe?>(null) }
    var code by remember { mutableStateOf<String?>(null) }
    var busy by remember { mutableStateOf(false) }
    var error by remember { mutableStateOf<String?>(null) }
    var job by remember { mutableStateOf<Job?>(null) }
    var attempt by remember { mutableIntStateOf(0) }
    val addressFocus = remember { FocusRequester() }
    val userFocus = remember { FocusRequester() }
    if (onBack != null) BackHandler(onBack = onBack)
    LaunchedEffect(Unit) { runCatching { addressFocus.requestFocus() } }
    LaunchedEffect(found) { if (found != null) runCatching { userFocus.requestFocus() } }
    fun attemptJob(block: suspend () -> Unit) {
        job?.cancel()
        val mine = ++attempt
        error = null
        code = null
        busy = true
        job = scope.launch {
            try {
                block()
            } catch (e: PlaivoException) {
                if (mine == attempt) {
                    error = e.message
                    code = null
                }
            } finally {
                if (mine == attempt) busy = false
            }
        }
    }
    fun probe() {
        if (address.isBlank()) return
        found = null
        attemptJob { found = io { core.jellyfinProbe(address) } }
    }
    fun signIn() {
        val server = found ?: return probe()
        if (username.isBlank()) {
            error = "Enter your username"
            return
        }
        attemptJob {
            io { core.jellyfinSignIn(server, username, password) }
            onAdded(server.name)
        }
    }
    fun quickConnect() {
        val server = found ?: return
        attemptJob {
            val qc = io { core.jellyfinQuickConnect(server) }
            code = qc.code
            busy = false
            do {
                delay(QUICK_CONNECT_POLL_MS)
            } while (!io { core.jellyfinQuickConnectPoll(server, qc.secret) })
            onAdded(server.name)
        }
    }
    Column(
        Modifier.fillMaxSize().verticalScroll(rememberScrollState()).padding(PaddingValues(horizontal = Dims.edge, vertical = 28.dp)),
        verticalArrangement = Arrangement.spacedBy(10.dp),
    ) {
        Heading("Add Library")
        Text("Enter the address you open Jellyfin with in a browser.", fontSize = 13.sp, color = Palette.muted)
        Row(horizontalArrangement = Arrangement.spacedBy(10.dp), verticalAlignment = Alignment.CenterVertically) {
            Field(address, {
                address = it
                job?.cancel()
                found = null
                code = null
                busy = false
            }, "Server address, like 192.168.1.10:8096", KeyboardType.Uri, ImeAction.Go, ::probe, Modifier.focusRequester(addressFocus))
            ActionButton("Connect", ::probe)
        }
        found?.let { server ->
            SettingsRow(Glyph.Server, server.name, "Jellyfin ${server.version} · ${server.uri}") { GlyphIcon(Glyph.Check, 20.dp, Palette.text) }
            Text("Sign in", fontSize = 13.sp, fontWeight = FontWeight.SemiBold, color = Palette.muted, modifier = Modifier.padding(top = 8.dp))
            Field(username, { username = it }, "Username", KeyboardType.Text, ImeAction.Next, {}, Modifier.focusRequester(userFocus))
            Field(password, { password = it }, "Password", KeyboardType.Password, ImeAction.Done, ::signIn, password = true)
            Row(horizontalArrangement = Arrangement.spacedBy(10.dp)) {
                ActionButton("Sign in", ::signIn)
                ActionButton("Use Quick Connect", ::quickConnect)
            }
            code?.let {
                Text("In Jellyfin on a signed-in device, open your profile → Quick Connect and enter this code for ${server.name}.", fontSize = 13.sp, color = Palette.muted)
                Text(it, fontSize = 40.sp, fontWeight = FontWeight.Bold, color = Palette.text, letterSpacing = 8.sp)
                Text("Waiting for approval…", fontSize = 13.sp, color = Palette.muted)
            }
        }
        if (busy) Text("Connecting…", fontSize = 13.sp, color = Palette.muted)
        error?.let { Text(it, fontSize = 13.sp, color = ERROR) }
    }
}

@Composable
private fun Field(
    value: String,
    onChange: (String) -> Unit,
    hint: String,
    type: KeyboardType,
    ime: ImeAction,
    onIme: () -> Unit,
    modifier: Modifier = Modifier,
    password: Boolean = false,
) {
    var focused by remember { mutableStateOf(false) }
    BasicTextField(
        value = value,
        onValueChange = onChange,
        singleLine = true,
        textStyle = TextStyle(fontSize = 16.sp, color = Palette.text),
        cursorBrush = SolidColor(Palette.text),
        visualTransformation = if (password) PasswordVisualTransformation() else VisualTransformation.None,
        keyboardOptions = KeyboardOptions(keyboardType = type, imeAction = ime, autoCorrectEnabled = false),
        keyboardActions = KeyboardActions(onAny = { onIme() }),
        modifier = modifier.widthIn(max = 520.dp).fillMaxWidth().onFocusChanged { focused = it.isFocused },
        decorationBox = { field ->
            Box(
                Modifier
                    .height(44.dp)
                    .background(Palette.surface, CircleShape)
                    .border(BorderStroke(if (focused) 2.dp else 1.dp, if (focused) Palette.text else Palette.outline), CircleShape)
                    .padding(horizontal = 16.dp),
                contentAlignment = Alignment.CenterStart,
            ) {
                if (value.isEmpty()) Text(hint, fontSize = 16.sp, color = Palette.muted)
                field()
            }
        },
    )
}

@Composable
fun ActionButton(text: String, onClick: () -> Unit) {
    Surface(
        onClick = onClick,
        modifier = Modifier.height(44.dp),
        shape = ClickableSurfaceDefaults.shape(CircleShape),
        colors = ClickableSurfaceDefaults.colors(containerColor = Palette.surface, contentColor = Palette.text, focusedContainerColor = Palette.tint(0.12f), focusedContentColor = Palette.text),
        scale = ClickableSurfaceDefaults.scale(focusedScale = 1f),
        border = ClickableSurfaceDefaults.border(
            border = Border(BorderStroke(1.dp, Palette.outline), shape = CircleShape),
            focusedBorder = Border(BorderStroke(2.dp, Palette.text), shape = CircleShape),
        ),
    ) {
        Box(Modifier.fillMaxHeight().padding(horizontal = 20.dp), contentAlignment = Alignment.Center) {
            Text(text, fontSize = 14.sp, fontWeight = FontWeight.SemiBold)
        }
    }
}
