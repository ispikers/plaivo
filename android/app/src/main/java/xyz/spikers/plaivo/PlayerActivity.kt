@file:OptIn(UnstableApi::class)

package xyz.spikers.plaivo

import android.app.UiModeManager
import android.content.pm.ActivityInfo
import android.content.res.Configuration
import android.os.Build
import android.os.Bundle
import android.os.SystemClock
import android.view.WindowManager
import android.widget.Toast
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.annotation.OptIn
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableFloatStateOf
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.core.net.toUri
import androidx.core.view.WindowCompat
import androidx.core.view.WindowInsetsCompat
import androidx.core.view.WindowInsetsControllerCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.lifecycleScope
import androidx.media3.common.C
import androidx.media3.common.MediaItem
import androidx.media3.common.MediaMetadata
import androidx.media3.common.MimeTypes
import androidx.media3.common.PlaybackException
import androidx.media3.common.Player
import androidx.media3.common.TrackSelectionOverride
import androidx.media3.common.Tracks
import androidx.media3.common.util.UnstableApi
import androidx.media3.datasource.HttpDataSource
import androidx.media3.exoplayer.DecoderReuseEvaluation
import androidx.media3.exoplayer.DefaultRenderersFactory
import androidx.media3.exoplayer.ExoPlayer
import androidx.media3.exoplayer.analytics.AnalyticsListener
import androidx.media3.exoplayer.mediacodec.MediaCodecUtil
import androidx.media3.ui.AspectRatioFrameLayout
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import uniffi.plaivo_ffi.Card as MediaCard
import uniffi.plaivo_ffi.LanguagePrefs
import uniffi.plaivo_ffi.LanguageRule
import uniffi.plaivo_ffi.PlaivoException
import uniffi.plaivo_ffi.Playback
import uniffi.plaivo_ffi.Scale as CoreScale
import uniffi.plaivo_ffi.Settings
import uniffi.plaivo_ffi.SkipMode
import uniffi.plaivo_ffi.SubtitleMode
import uniffi.plaivo_ffi.TrackInfo
import uniffi.plaivo_ffi.WatchCommand
import uniffi.plaivo_ffi.WatchFeed
import uniffi.plaivo_ffi.pickTracks
import uniffi.plaivo_ffi.setPlaybackQuality

enum class Scale(val label: String, val glyph: Glyph, val resize: Int) {
    Fit("Letterbox", Glyph.ScaleFit, AspectRatioFrameLayout.RESIZE_MODE_FIT),
    Fill("Fill screen", Glyph.ScaleFill, AspectRatioFrameLayout.RESIZE_MODE_ZOOM),
    Stretch("Stretch", Glyph.ScaleStretch, AspectRatioFrameLayout.RESIZE_MODE_FILL);

    fun next() = entries[(ordinal + 1) % entries.size]
}

private const val UP_NEXT_LEAD_MS = 30_000L
private const val WATCH_FEED_MS = 500L
private const val WATCH_SEEK_MS = 2000L
private val reports = Dispatchers.IO.limitedParallelism(1)

val SPEEDS = listOf(0.5f, 0.75f, 1f, 1.25f, 1.5f, 1.75f, 2f)

fun fmtSpeed(s: Float) = if (s == 1f) "Normal" else "${s.toString().removeSuffix(".0")}×"

data class TrackChoice(
    val group: Tracks.Group?,
    val index: Int,
    val label: String,
    val detail: String,
    val selected: Boolean,
)

class PlayerActivity : ComponentActivity() {
    lateinit var exo: ExoPlayer
        private set

    var playback by mutableStateOf<Playback?>(null)
        private set

    var queue by mutableStateOf<List<MediaCard>>(emptyList())
        private set

    var scale by mutableStateOf(Scale.Fit)
        private set

    var speed by mutableFloatStateOf(1f)
        private set

    var tracksVersion by mutableIntStateOf(0)
        private set

    var videoDecoder by mutableStateOf("")
        private set

    var quality by mutableIntStateOf(0)
        private set

    val qualities by lazy { core.qualities() }
    val prefs: Settings = core.settings()
    var upNextCancelled by mutableStateOf(false)
        private set

    var advancing = false
        private set

    private var upNextAnchor: ULong? = null
    val autoSkipped = mutableSetOf<Int>()
    private var queueShow: String? = null
    private var fellBack = false
    private var loading: Job? = null
    private val audioCodecs by lazy {
        val dolby =
            listOf("ac3" to MimeTypes.AUDIO_AC3, "eac3" to MimeTypes.AUDIO_E_AC3).filter { (_, mime) ->
                runCatching { MediaCodecUtil.getDecoderInfos(mime, false, false).isNotEmpty() }.getOrDefault(false)
            }
        (listOf("aac", "mp3") + dolby.map { it.first }).joinToString(",")
    }
    private var version = 0
    private var manualTracks = false
    private var autoTracks = false
    private var firstFrame = false
    private var watchAttached = false
    private var watchFedAt = 0L
    private var watchSeek: Pair<Long, Long>? = null

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        val touch = getSystemService(UiModeManager::class.java).currentModeType != Configuration.UI_MODE_TYPE_TELEVISION
        if (touch) {
            requestedOrientation = ActivityInfo.SCREEN_ORIENTATION_SENSOR_LANDSCAPE
            WindowCompat.setDecorFitsSystemWindows(window, false)
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.P)
                window.attributes =
                    window.attributes.apply {
                        layoutInDisplayCutoutMode = WindowManager.LayoutParams.LAYOUT_IN_DISPLAY_CUTOUT_MODE_SHORT_EDGES
                    }
            WindowCompat.getInsetsController(window, window.decorView).run {
                systemBarsBehavior = WindowInsetsControllerCompat.BEHAVIOR_SHOW_TRANSIENT_BARS_BY_SWIPE
                hide(WindowInsetsCompat.Type.systemBars())
            }
        }
        val server = intent.getStringExtra("server") ?: return finish()
        val key = intent.getStringExtra("key") ?: return finish()
        version = intent.getIntExtra("version", 0)
        quality = prefs.quality.toInt()
        scale = Scale.entries[prefs.scale.ordinal]
        val renderers =
            DefaultRenderersFactory(this).setMediaCodecSelector { mime, secure, tunneling ->
                // the emulator's host-backed decoders fail mid-stream; its software ones work
                MediaCodecUtil.getDecoderInfos(mime, secure, tunneling).filterNot { it.name.startsWith("c2.goldfish.") }
            }
        exo = ExoPlayer.Builder(this, renderers).build()
        Watch.player = this
        exo.addListener(
            object : Player.Listener {
                override fun onIsPlayingChanged(isPlaying: Boolean) {
                    report(if (isPlaying) "playing" else "paused")
                    feedWatch(force = true)
                }

                override fun onRenderedFirstFrame() {
                    firstFrame = true
                    feedWatch(force = true)
                }

                override fun onTracksChanged(tracks: Tracks) {
                    tracksVersion++
                    val p = playback
                    if (autoTracks && p != null && !tracks.isEmpty) {
                        autoTracks = false
                        autoSelectTracks(p.languages, p.languageRule)
                    }
                    val unsupported =
                        listOf(C.TRACK_TYPE_VIDEO, C.TRACK_TYPE_AUDIO).filter {
                            tracks.containsType(it) && !tracks.isTypeSupported(it)
                        }
                    if (unsupported.isNotEmpty()) fallBack(if (C.TRACK_TYPE_VIDEO in unsupported) "video" else "audio")
                }

                override fun onPlaybackStateChanged(state: Int) {
                    feedWatch(force = true)
                    if (state != Player.STATE_ENDED || advancing || watching()) return
                    val next = neighbor(1)?.takeIf { prefs.autoplayNext && !upNextCancelled }
                    if (next != null) playNext() else finish()
                }

                override fun onPlayerError(error: PlaybackException) {
                    (error.cause as? HttpDataSource.InvalidResponseCodeException)?.let {
                        android.util.Log.w(
                            "Plaivo",
                            "HTTP ${it.responseCode} ${it.dataSpec.uri.buildUpon().clearQuery()}: ${String(it.responseBody).take(300)}",
                        )
                    }
                    if (fellBack && playback?.transcodeSession == null) return
                    if (error.errorCode in 3000..5999 && fallBack(error.errorCodeName)) return
                    Toast.makeText(this@PlayerActivity, "Playback error: ${error.errorCodeName}", Toast.LENGTH_LONG)
                        .show()
                    finish()
                }
            }
        )
        exo.addAnalyticsListener(
            object : AnalyticsListener {
                override fun onVideoDecoderInitialized(
                    eventTime: AnalyticsListener.EventTime,
                    decoderName: String,
                    initializedTimestampMs: Long,
                    initializationDurationMs: Long,
                ) {
                    videoDecoder = decoderName
                }

                override fun onVideoInputFormatChanged(
                    eventTime: AnalyticsListener.EventTime,
                    format: androidx.media3.common.Format,
                    decoderReuseEvaluation: DecoderReuseEvaluation?,
                ) {
                    tracksVersion++
                }
            }
        )
        setContent { CompositionLocalProvider(LocalTouch provides touch) { PlaivoTheme { PlayerScreen(this) } } }
        play(server, key, if (intent.getBooleanExtra("fromStart", false)) 0L else null)
        lifecycleScope.launch {
            while (true) {
                delay(10_000)
                if (exo.isPlaying) report("playing")
            }
        }
    }

    private fun fallBack(reason: String): Boolean {
        val p = playback ?: return false
        if (p.transcodeSession != null || fellBack) return false
        fellBack = true
        android.util.Log.i("Plaivo", "direct play failed ($reason), transcoding ${p.key}")
        Toast.makeText(this, "This device can't play the file directly. Transcoding…", Toast.LENGTH_SHORT).show()
        val at = exo.currentPosition.coerceAtLeast(0)
        exo.stop()
        play(p.server, p.key, at, forceTranscode = true)
        return true
    }

    fun play(server: String, key: String, startAt: Long? = null, forceTranscode: Boolean = false) {
        if (playback != null && playback?.key != key) version = 0
        loading?.cancel()
        loading =
            lifecycleScope.launch {
                val p =
                    try {
                        io { core.playback(server, key, forceTranscode, version.toUInt(), audioCodecs) }
                    } catch (e: PlaivoException) {
                        Watch.failed(server, key)
                        if (advancing) upNextCancelled = true
                        advancing = false
                        Toast.makeText(this@PlayerActivity, e.message, Toast.LENGTH_LONG).show()
                        if (playback == null || forceTranscode) finish()
                        return@launch
                    }
                val previous = playback
                if (previous?.key != p.key) fellBack = false
                val session = previous?.transcodeSession
                if (previous != null && previous.key != p.key && session == null) report("stopped")
                if (previous != null && session != null) {
                    val time = exo.currentPosition.coerceAtLeast(0).toULong()
                    exo.stop()
                    // Plex answers 400 to a new transcode while the old playback is still reported as active
                    io {
                        core.report(previous.server, previous.key, "stopped", time, previous.durationMs, session)
                        core.stopTranscode(previous.server, session)
                    }
                }
                playback = p
                quality = p.qualityKbps.toInt()
                if (previous?.key != p.key) manualTracks = false
                if (!manualTracks) {
                    autoTracks = true
                    val l = p.languages
                    exo.trackSelectionParameters =
                        exo.trackSelectionParameters
                            .buildUpon()
                            .clearOverrides()
                            .setTrackTypeDisabled(C.TRACK_TYPE_TEXT, false)
                            .setPreferredAudioLanguage(l.audio.ifEmpty { null })
                            .setPreferredTextLanguage(
                                l.subtitles.ifEmpty { null }.takeIf { l.mode != SubtitleMode.OFF }
                            )
                            .build()
                }
                advancing = false
                upNextCancelled = false
                upNextAnchor = null
                autoSkipped.clear()
                firstFrame = false
                exo.setMediaItem(mediaItem(p), startAt ?: p.startMs.toLong())
                exo.prepare()
                exo.playWhenReady = true
                Watch.loaded(p.server, p.key, p.durationMs.toLong())
                val show = p.showKey
                if (show == null) {
                    queueShow = null
                    queue = emptyList()
                } else if (show != queueShow) {
                    queue = io { runCatching { core.queue(p.server, show) } }.getOrDefault(emptyList())
                    queueShow = show
                }
            }
    }

    fun changeQuality(kbps: Int) {
        if (kbps == quality) return
        quality = kbps
        editSettings { setPlaybackQuality(it, playback?.library ?: "", kbps.toUInt()) }
        val p = playback ?: return
        play(p.server, p.key, exo.currentPosition.coerceAtLeast(0), forceTranscode = fellBack)
    }

    private fun stopTranscode(p: Playback) {
        val session = p.transcodeSession ?: return
        background.launch { core.stopTranscode(p.server, session) }
    }

    fun neighbor(step: Int): MediaCard? {
        val i = queue.indexOfFirst { it.key == playback?.key }
        return if (i < 0) null else queue.getOrNull(i + step)
    }

    fun playNext() {
        val next = neighbor(1) ?: return
        advancing = true
        play(next.server, next.key)
    }

    fun cancelUpNext() {
        upNextCancelled = true
    }

    fun upNextIn(positionMs: Long, durationMs: Long): Double? {
        val p = playback ?: return null
        if (watching() || upNextCancelled || advancing || durationMs <= 0) return null
        val credits = p.markers.firstOrNull { it.kind == "credits" }?.startMs
        if (positionMs < minOf(credits?.toLong() ?: durationMs, durationMs - UP_NEXT_LEAD_MS)) {
            upNextAnchor = null
            return null
        }
        if (neighbor(1) == null) return null
        val next = core.upNextIn(positionMs.coerceAtLeast(0).toULong(), durationMs.toULong(), credits, upNextAnchor)
        upNextAnchor = next?.anchorMs
        return next?.leftS
    }

    fun autoSkip(positionMs: Long): String? {
        val markers = playback?.markers?.takeIf { !watching() } ?: return null
        val i =
            markers.indexOfFirst {
                it.mode == SkipMode.AUTO && positionMs >= it.startMs.toLong() && positionMs < it.endMs.toLong() - 1000
            }
        if (i < 0 || !autoSkipped.add(i)) return null
        exo.seekTo(markers[i].endMs.toLong())
        return markers[i].label.replaceFirst("Skip", "Skipped")
    }

    fun togglePause() = setPlaying(!exo.playWhenReady)

    fun setPlaying(on: Boolean) {
        if (!watching()) exo.playWhenReady = on else if (on) core.watchPlay() else core.watchPause()
    }

    fun seekBy(ms: Long) {
        val duration = exo.duration.takeIf { it > 0 } ?: Long.MAX_VALUE
        val from =
            watchSeek?.takeIf { (_, at) -> SystemClock.elapsedRealtime() - at < WATCH_SEEK_MS }?.first
                ?: exo.currentPosition
        seekTo((from + ms).coerceIn(0, duration))
    }

    fun seekTo(ms: Long) {
        if (!watching()) return exo.seekTo(ms)
        watchSeek = ms to SystemClock.elapsedRealtime()
        core.watchSeek(ms)
    }

    fun watching() = watchEpoch() != null

    private fun watchEpoch(): Long? {
        val p = playback ?: return null
        if (!started()) return null
        return Watch.epochFor(p.server, p.key)
    }

    fun started() = lifecycle.currentState.isAtLeast(Lifecycle.State.STARTED)

    fun watch(b: Watch.Bound) {
        val p = playback
        if (p?.server == b.server && p.key == b.key) Watch.loaded(p.server, p.key, p.durationMs.toLong())
        else play(b.server, b.key)
    }

    fun feedWatch(force: Boolean = false) {
        val epoch = watchEpoch()
        if (epoch == null) {
            if (watchAttached) {
                core.watchDetach()
                exo.setPlaybackSpeed(speed)
            }
            watchAttached = false
            return
        }
        val now = SystemClock.elapsedRealtime()
        if (!force && now - watchFedAt < WATCH_FEED_MS) return
        watchFedAt = now
        watchAttached = true
        val state = exo.playbackState
        val loading = !firstFrame || state == Player.STATE_IDLE
        core.watchFeed(
            epoch,
            WatchFeed(
                exo.currentPosition.coerceAtLeast(0),
                exo.duration.takeIf { it > 0 },
                !exo.playWhenReady,
                !loading && state == Player.STATE_BUFFERING,
                loading,
            ),
        )
    }

    fun follow(c: WatchCommand) {
        if (!watchAttached) return
        when (c) {
            is WatchCommand.Pause -> exo.playWhenReady = false
            is WatchCommand.Play -> exo.playWhenReady = true
            is WatchCommand.Seek -> exo.seekTo(c.positionMs)
            is WatchCommand.SetRate -> {
                exo.setPlaybackSpeed(c.rate.toFloat())
                if (c.rate.toFloat() in SPEEDS) speed = c.rate.toFloat()
            }
        }
        feedWatch(force = true)
    }

    fun cycleScale(): Scale {
        scale = scale.next()
        editSettings { it.copy(scale = CoreScale.entries[scale.ordinal]) }
        return scale
    }

    fun changeSpeed(s: Float) {
        if (watching()) return core.watchSetRate(s.toDouble())
        speed = s
        exo.setPlaybackSpeed(s)
    }

    fun tracks(type: Int): List<TrackChoice> =
        exo.currentTracks.groups
            .filter { it.type == type }
            .flatMap { g ->
                (0 until g.length)
                    .filter { g.isTrackSupported(it) }
                    .map { i ->
                        val f = g.getTrackFormat(i)
                        val language =
                            f.language
                                ?.takeIf { it != "und" }
                                ?.let { java.util.Locale.forLanguageTag(it).displayLanguage }
                        val label = f.label ?: language ?: "Track ${i + 1}"
                        val detail =
                            listOfNotNull(
                                    codecName(f.sampleMimeType ?: f.codecs),
                                    channelLayout(f.channelCount).takeIf { type == C.TRACK_TYPE_AUDIO },
                                    language.takeIf { f.label != null },
                                    "Forced".takeIf { f.selectionFlags and C.SELECTION_FLAG_FORCED != 0 },
                                )
                                .filter { it.isNotEmpty() }
                                .joinToString(" · ")
                        TrackChoice(g, i, label, detail, g.isTrackSelected(i))
                    }
            }

    private fun TrackChoice.info(): TrackInfo {
        val f = group!!.getTrackFormat(index)
        return TrackInfo(
            f.language ?: "",
            f.selectionFlags and C.SELECTION_FLAG_FORCED != 0,
            f.selectionFlags and C.SELECTION_FLAG_DEFAULT != 0,
        )
    }

    private fun autoSelectTracks(languages: LanguagePrefs, rule: LanguageRule?) {
        val audio = tracks(C.TRACK_TYPE_AUDIO)
        val text = tracks(C.TRACK_TYPE_TEXT)
        val pick = pickTracks(audio.map { it.info() }, text.map { it.info() }, languages, rule)
        val params = exo.trackSelectionParameters.buildUpon()
        pick.audio
            ?.let { audio[it.toInt()] }
            ?.let { params.setOverrideForType(TrackSelectionOverride(it.group!!.mediaTrackGroup, it.index)) }
        val sub = pick.text?.let { text[it.toInt()] }
        params.setTrackTypeDisabled(C.TRACK_TYPE_TEXT, sub == null)
        sub?.let { params.setOverrideForType(TrackSelectionOverride(it.group!!.mediaTrackGroup, it.index)) }
        exo.trackSelectionParameters = params.build()
    }

    fun select(type: Int, choice: TrackChoice?) {
        manualTracks = true
        val params = exo.trackSelectionParameters.buildUpon()
        if (choice?.group == null) {
            params.setTrackTypeDisabled(type, true)
        } else {
            params
                .setTrackTypeDisabled(type, false)
                .setOverrideForType(TrackSelectionOverride(choice.group.mediaTrackGroup, choice.index))
        }
        exo.trackSelectionParameters = params.build()
    }

    fun textDisabled() = C.TRACK_TYPE_TEXT in exo.trackSelectionParameters.disabledTrackTypes

    fun fileInfo(): List<Pair<String, String>> {
        val p = playback ?: return emptyList()
        val v = exo.videoFormat
        val a = exo.audioFormat
        val color =
            v?.colorInfo?.let {
                when (it.colorTransfer) {
                    C.COLOR_TRANSFER_ST2084 -> "HDR10 (PQ)"
                    C.COLOR_TRANSFER_HLG -> "HLG"
                    else -> "SDR"
                }
            } ?: ""
        val rows =
            listOf(
                "Playback" to if (p.transcodeSession != null) "Transcode · ${p.transcodeLabel}" else "Direct play",
                "File" to p.file,
                "Container" to
                    join(p.container.uppercase(), if (p.bitrateKbps > 0uL) fmtBitrate(p.bitrateKbps.toLong()) else ""),
                "Video" to
                    join(
                        codecName(v?.sampleMimeType),
                        v?.takeIf { it.width > 0 }?.let { "${it.width}×${it.height}" } ?: "",
                        v?.frameRate
                            ?.takeIf { it > 0 }
                            ?.let { "${"%.3f".format(it).trimEnd('0').trimEnd('.', ',')} fps" } ?: "",
                        v?.bitrate?.takeIf { it > 0 }?.let { fmtBitrate(it / 1000L) } ?: "",
                    ),
                "Color" to color,
                "Decoder" to videoDecoder,
                "Audio" to
                    join(
                        codecName(a?.sampleMimeType),
                        channelLayout(a?.channelCount ?: 0),
                        a?.sampleRate?.takeIf { it > 0 }?.let { "${it / 1000.0} kHz" } ?: "",
                    ),
                "Subtitles" to
                    tracks(C.TRACK_TYPE_TEXT).let { subs ->
                        subs.firstOrNull { it.selected }?.let { join(it.label, it.detail) }
                            ?: if (subs.isEmpty()) "" else "Off"
                    },
            )
        return rows.filter { it.second.isNotEmpty() }
    }

    override fun onStart() {
        super.onStart()
        val b = Watch.item ?: return
        val p = playback ?: return
        if (p.server != b.server || p.key != b.key) watch(b)
    }

    override fun onStop() {
        super.onStop()
        feedWatch()
        exo.pause()
    }

    override fun onDestroy() {
        if (Watch.player === this) Watch.player = null
        report("stopped")
        playback?.let { stopTranscode(it) }
        if (::exo.isInitialized) exo.release()
        super.onDestroy()
    }

    private fun report(state: String) {
        val p = playback ?: return
        val time = exo.currentPosition.coerceAtLeast(0).toULong()
        background.launch(reports) { core.report(p.server, p.key, state, time, p.durationMs, p.transcodeSession) }
    }

    private fun mediaItem(p: Playback): MediaItem {
        val subtitles =
            p.subtitles.mapNotNull { s ->
                val mime =
                    when (s.codec.lowercase()) {
                        "srt",
                        "subrip" -> MimeTypes.APPLICATION_SUBRIP
                        "ass",
                        "ssa" -> MimeTypes.TEXT_SSA
                        "vtt",
                        "webvtt" -> MimeTypes.TEXT_VTT
                        else -> return@mapNotNull null
                    }
                MediaItem.SubtitleConfiguration.Builder(s.url.toUri())
                    .setMimeType(mime)
                    .setLanguage(s.language)
                    .setLabel(s.title)
                    .setSelectionFlags(if (s.forced) C.SELECTION_FLAG_FORCED else 0)
                    .build()
            }
        return MediaItem.Builder()
            .setUri(p.url)
            .apply { if (p.transcodeSession != null) setMimeType(MimeTypes.APPLICATION_M3U8) }
            .setSubtitleConfigurations(subtitles)
            .setMediaMetadata(MediaMetadata.Builder().setTitle(p.title).setSubtitle(p.subtitle).build())
            .build()
    }
}

private fun join(vararg parts: String) = parts.filter { it.isNotEmpty() }.joinToString(" · ")

fun fmtBitrate(kbps: Long) = if (kbps >= 1000) "%.1f Mbps".format(kbps / 1000.0) else "$kbps kbps"

private fun channelLayout(n: Int) =
    when (n) {
        0 -> ""
        1 -> "Mono"
        2 -> "Stereo"
        6 -> "5.1"
        8 -> "7.1"
        else -> "$n ch"
    }

private fun codecName(mime: String?): String =
    when (mime?.substringAfter('/')?.lowercase()) {
        null,
        "" -> ""
        "avc" -> "H.264"
        "hevc" -> "HEVC"
        "dolby-vision" -> "Dolby Vision"
        "x-vnd.on2.vp9" -> "VP9"
        "av01" -> "AV1"
        "mp4a-latm" -> "AAC"
        "ac3" -> "AC3"
        "eac3" -> "EAC3"
        "eac3-joc" -> "EAC3 Atmos"
        "true-hd" -> "TrueHD"
        "vnd.dts",
        "vnd.dts.hd" -> "DTS"
        "opus" -> "Opus"
        "flac" -> "FLAC"
        "x-subrip" -> "SRT"
        "x-ssa" -> "ASS"
        "vtt" -> "WebVTT"
        "pgs" -> "PGS"
        else -> mime.substringAfter('/').removePrefix("x-").uppercase()
    }
