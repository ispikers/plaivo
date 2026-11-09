package xyz.spikers.plaivo

import android.Manifest
import android.app.Notification
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationChannelCompat
import androidx.core.app.NotificationCompat
import androidx.core.app.NotificationManagerCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import kotlinx.coroutines.launch
import uniffi.plaivo_ffi.Download
import uniffi.plaivo_ffi.DownloadState

class DownloadService : Service() {
    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        val list = DownloadQueue.entries.value
        ServiceCompat.startForeground(
            this,
            NOTIFICATION,
            notification(this, list),
            ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC,
        )
        running = this
        if (intent?.action == PAUSE) background.launch { core.pauseAllDownloads() } else if (!active(list)) stopSelf()
        return START_NOT_STICKY
    }

    override fun onTimeout(startId: Int, fgsType: Int) {
        blocked = true
        stopSelf()
    }

    override fun onDestroy() {
        running = null
        super.onDestroy()
    }

    companion object {
        private const val CHANNEL = "downloads"
        private const val NOTIFICATION = 1
        private const val PAUSE = "pause"

        @Volatile private var running: DownloadService? = null

        @Volatile private var blocked = false

        fun unblock() {
            blocked = false
        }

        fun createChannel(context: Context) {
            NotificationManagerCompat.from(context)
                .createNotificationChannel(
                    NotificationChannelCompat.Builder(CHANNEL, NotificationManagerCompat.IMPORTANCE_LOW)
                        .setName("Downloads")
                        .build()
                )
        }

        fun sync(context: Context, list: List<Download>) {
            val service = running
            when {
                !active(list) -> service?.stopSelf()
                service == null ->
                    if (!blocked)
                        runCatching {
                                ContextCompat.startForegroundService(
                                    context,
                                    Intent(context, DownloadService::class.java),
                                )
                            }
                            .onFailure {
                                blocked = true
                                android.util.Log.w(
                                    "Plaivo",
                                    "download service not started until the app is opened again: $it",
                                )
                            }
                Build.VERSION.SDK_INT < 33 ||
                    ContextCompat.checkSelfPermission(context, Manifest.permission.POST_NOTIFICATIONS) ==
                        PackageManager.PERMISSION_GRANTED ->
                    NotificationManagerCompat.from(context).notify(NOTIFICATION, notification(context, list))
            }
        }

        private fun active(list: List<Download>): Boolean {
            val servers by lazy { core.servers().map { it.id }.toSet() }
            return list.any {
                it.state == DownloadState.DOWNLOADING || (it.state == DownloadState.QUEUED && it.server in servers)
            }
        }

        private fun notification(context: Context, list: List<Download>): Notification {
            val current = list.firstOrNull { it.state == DownloadState.DOWNLOADING }
            val waiting = list.count { it.state == DownloadState.QUEUED }
            val percent = current?.takeIf { it.total > 0uL }?.let { (it.bytes * 100uL / it.total).toInt() }
            val pause =
                PendingIntent.getService(
                    context,
                    0,
                    Intent(context, DownloadService::class.java).setAction(PAUSE),
                    PendingIntent.FLAG_IMMUTABLE,
                )
            val open =
                PendingIntent.getActivity(
                    context,
                    0,
                    Intent(context, MainActivity::class.java),
                    PendingIntent.FLAG_IMMUTABLE,
                )
            return NotificationCompat.Builder(context, CHANNEL)
                .setSmallIcon(android.R.drawable.stat_sys_download)
                .setContentTitle(
                    current?.let { listOf(it.title, it.subtitle).filter(String::isNotEmpty).joinToString(" · ") }
                        ?: "Waiting to download"
                )
                .setContentText(if (waiting > 0) "$waiting more in the queue" else null)
                .setProgress(100, percent ?: 0, percent == null)
                .setOngoing(true)
                .setOnlyAlertOnce(true)
                .setSilent(true)
                .setContentIntent(open)
                .addAction(android.R.drawable.ic_media_pause, "Pause all", pause)
                .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
                .build()
        }
    }
}
