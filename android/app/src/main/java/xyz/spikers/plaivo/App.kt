package xyz.spikers.plaivo

import android.app.Application
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext
import uniffi.plaivo_ffi.Plaivo
import uniffi.plaivo_ffi.Settings

class App : Application() {
    val core by lazy { Plaivo(filesDir.absolutePath, cacheDir.resolve("core").absolutePath) }

    override fun onCreate() {
        super.onCreate()
        instance = this
        Palette.theme = core.settings().theme
        DownloadQueue.start(this)
        Watch.start()
    }

    companion object {
        lateinit var instance: App
    }
}

val core: Plaivo
    get() = App.instance.core

val background = CoroutineScope(SupervisorJob() + Dispatchers.IO)

private val settingsWriter = Dispatchers.IO.limitedParallelism(1)
private val settingsLock = Any()
private var pendingSettings: Settings? = null

fun editSettings(change: (Settings) -> Settings): Settings =
    synchronized(settingsLock) {
        val next = change(pendingSettings ?: core.settings())
        pendingSettings = next
        Palette.theme = next.theme
        background.launch(settingsWriter) {
            core.setSettings(next)
            synchronized(settingsLock) { if (pendingSettings === next) pendingSettings = null }
        }
        next
    }

suspend fun <T> io(block: () -> T): T = withContext(Dispatchers.IO) { block() }
