package xyz.spikers.plaivo

import android.content.Intent
import android.widget.Toast
import androidx.activity.compose.BackHandler
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.verticalScroll
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.focus.FocusRequester
import androidx.compose.ui.focus.focusRequester
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.net.toUri
import androidx.tv.material3.Text
import kotlinx.coroutines.delay
import uniffi.plaivo_ffi.PlaivoException
import uniffi.plaivo_ffi.Service
import uniffi.plaivo_ffi.TrackerPoll
import uniffi.plaivo_ffi.TrackerSignIn

val Service.title
    get() =
        when (this) {
            Service.TRAKT -> "Trakt"
            Service.MAL -> "MyAnimeList"
        }

private fun failure(reason: String) =
    when (reason) {
        "expired" -> "The code expired. Press Back and try again."
        "denied" -> "Access was denied."
        else -> "Sign-in failed: $reason"
    }

@Composable
fun TrackerSignInScreen(service: Service, onClose: () -> Unit) {
    val context = LocalContext.current
    BackHandler(onBack = onClose)
    var sign by remember { mutableStateOf<TrackerSignIn?>(null) }
    var status by remember { mutableStateOf("Connecting…") }
    val cancel = remember { FocusRequester() }
    LaunchedEffect(Unit) { runCatching { cancel.requestFocus() } }
    LaunchedEffect(Unit) {
        val s =
            try {
                io { core.beginTrackerSignIn(service) }
            } catch (e: PlaivoException) {
                status = "Could not start sign-in: ${e.message}"
                return@LaunchedEffect
            }
        sign = s
        status = "Waiting for approval…"
        var interval = s.intervalSecs.toLong()
        val expires = System.currentTimeMillis() + s.expiresInSecs.toLong() * 1000
        while (System.currentTimeMillis() < expires) {
            delay(interval * 1000)
            when (val r = io { s.session.poll() }) {
                is TrackerPoll.Pending -> interval = r.intervalSecs.toLong()
                is TrackerPoll.Done -> {
                    Toast.makeText(context, "Connected to ${service.title}", Toast.LENGTH_SHORT).show()
                    return@LaunchedEffect onClose()
                }
                is TrackerPoll.Failed -> {
                    status = failure(r.reason)
                    return@LaunchedEffect
                }
            }
        }
        status = failure("expired")
    }
    val touch = LocalTouch.current
    Column(
        Modifier.fillMaxSize()
            .then(if (touch) Modifier.verticalScroll(rememberScrollState()).padding(Dims.gutter) else Modifier),
        verticalArrangement = Arrangement.spacedBy(24.dp, Alignment.CenterVertically),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        Heading("Connect ${service.title}")
        val s = sign
        if (touch) {
            SignInCode(s, Modifier.fillMaxWidth())
            if (s != null)
                ActionButton(
                    "Open in browser",
                    { runCatching { context.startActivity(Intent(Intent.ACTION_VIEW, s.url.toUri())) } },
                )
            SignInQr(s)
        } else
            Row(horizontalArrangement = Arrangement.spacedBy(56.dp), verticalAlignment = Alignment.CenterVertically) {
                SignInCode(s, Modifier.width(420.dp))
                SignInQr(s)
            }
        Text(status, fontSize = 14.sp, color = Palette.muted)
        Box(Modifier.focusRequester(cancel)) { ActionButton("Cancel", onClick = onClose) }
    }
}

@Composable
private fun SignInCode(s: TrackerSignIn?, modifier: Modifier) {
    Column(
        modifier,
        verticalArrangement = Arrangement.spacedBy(8.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
    ) {
        val code = s?.userCode
        Text(
            if (code != null) "On your phone or computer, go to" else "Scan the code with your phone, or open",
            fontSize = 16.sp,
            color = Palette.muted,
        )
        Text(
            s?.url?.removePrefix("https://") ?: "…",
            fontSize = if (code != null) 24.sp else 14.sp,
            color = Palette.text,
            textAlign = TextAlign.Center,
            maxLines = 3,
        )
        if (code != null) {
            Text("and enter this code", fontSize = 16.sp, color = Palette.muted)
            Text(code, fontSize = 44.sp, fontWeight = FontWeight.Bold, color = Palette.text, letterSpacing = 8.sp)
        }
    }
}

@Composable
private fun SignInQr(s: TrackerSignIn?) {
    if (s != null) QrCode(s.qrSize.toInt(), s.qr, Modifier.size(220.dp))
    else Box(Modifier.size(220.dp).background(Palette.surface, RoundedCornerShape(12.dp)))
}
