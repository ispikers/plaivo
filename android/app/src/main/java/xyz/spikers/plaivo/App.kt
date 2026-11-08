package xyz.spikers.plaivo

import android.app.Application
import androidx.core.content.edit
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
        migratePrefs()
        DownloadQueue.start(this)
        Watch.start()
    }

    private fun migratePrefs() {
        val prefs = getSharedPreferences("settings", MODE_PRIVATE)
        if (prefs.getBoolean(MIGRATED, false)) return
        background.launch { cacheDir.resolve("meta").deleteRecursively() }
        val saved = core.migrateAndroidPrefs(
            prefs.getBoolean("sidebar_compact", true),
            prefs.getString("home_server", null),
            if (prefs.contains("quality")) prefs.getInt("quality", 0) else null,
            prefs.getString("scale", null),
        )
        if (!saved) return
        prefs.edit {
            clear()
            putBoolean(MIGRATED, true)
        }
    }

    companion object {
        lateinit var instance: App
        private const val MIGRATED = "migrated_to_core"
    }
}

val core: Plaivo get() = App.instance.core

val background = CoroutineScope(SupervisorJob() + Dispatchers.IO)

private val settingsWriter = Dispatchers.IO.limitedParallelism(1)
private val settingsLock = Any()
private var pendingSettings: Settings? = null

fun editSettings(change: (Settings) -> Settings): Settings = synchronized(settingsLock) {
    val next = change(pendingSettings ?: core.settings())
    pendingSettings = next
    background.launch(settingsWriter) {
        core.setSettings(next)
        synchronized(settingsLock) { if (pendingSettings === next) pendingSettings = null }
    }
    next
}

suspend fun <T> io(block: () -> T): T = withContext(Dispatchers.IO) { block() }
