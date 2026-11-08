package xyz.spikers.plaivo

import android.app.Application
import androidx.core.content.edit
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.withContext
import uniffi.plaivo_ffi.Plaivo
import uniffi.plaivo_ffi.Settings

class App : Application() {
    val core by lazy { Plaivo(filesDir.absolutePath, cacheDir.absolutePath) }

    override fun onCreate() {
        super.onCreate()
        instance = this
        migratePrefs()
    }

    private fun migratePrefs() {
        val prefs = getSharedPreferences("settings", MODE_PRIVATE)
        if (prefs.getBoolean(MIGRATED, false)) return
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

fun editSettings(change: (Settings) -> Settings): Settings = change(core.settings()).also { core.setSettings(it) }

suspend fun <T> io(block: () -> T): T = withContext(Dispatchers.IO) { block() }
