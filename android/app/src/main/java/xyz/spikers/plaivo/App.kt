package xyz.spikers.plaivo

import android.app.Application
import android.content.SharedPreferences
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.withContext
import uniffi.plaivo_ffi.Plaivo

class App : Application() {
    val core by lazy { Plaivo(filesDir.absolutePath, cacheDir.absolutePath) }
    val prefs: SharedPreferences by lazy { getSharedPreferences("settings", MODE_PRIVATE) }

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    companion object {
        lateinit var instance: App
    }
}

val core: Plaivo get() = App.instance.core

val background = CoroutineScope(SupervisorJob() + Dispatchers.IO)

const val PREF_SIDEBAR_COMPACT = "sidebar_compact"
const val PREF_HOME_SERVER = "home_server"
const val PREF_SCALE = "scale"
const val PREF_QUALITY = "quality"

suspend fun <T> io(block: () -> T): T = withContext(Dispatchers.IO) { block() }
