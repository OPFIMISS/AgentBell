package com.agentbell.mobile

import android.app.*
import android.content.Context
import android.content.Intent
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.os.SystemClock
import android.util.Log
import org.json.JSONObject
import java.net.HttpURLConnection
import java.net.URL
import java.net.URLEncoder
import java.util.concurrent.atomic.AtomicBoolean

class AgentBellService : Service() {
    private val running = AtomicBoolean(false)
    override fun onBind(intent: Intent?): IBinder? = null
    override fun onCreate() { super.onCreate(); createChannels(); startForeground(810, statusNotification("AgentBell 正在连接", "等待电脑响应")); log("foreground service started") }
    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int { if (running.compareAndSet(false, true)) Thread { pollLoop() }.start(); return START_STICKY }
    override fun onTaskRemoved(rootIntent: Intent?) { scheduleRestart(); super.onTaskRemoved(rootIntent) }
    override fun onDestroy() { running.set(false); scheduleRestart(); super.onDestroy() }

    private fun pollLoop() {
        val prefs = getSharedPreferences("agentbell", Context.MODE_PRIVATE)
        while (running.get() && prefs.getBoolean("enabled", false)) {
            val wakeLock = (getSystemService(Context.POWER_SERVICE) as PowerManager).newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "AgentBell:poll")
            try {
                wakeLock.acquire(8000)
                val base = prefs.getString("url", "")!!.trimEnd('/'); val token = prefs.getString("token", "")!!; val cursor = prefs.getString("cursor", "")!!
                if (base.isNotEmpty() && token.isNotEmpty()) {
                    val endpoint = "$base/api/events/poll?token=${URLEncoder.encode(token, "UTF-8")}&after_id=${URLEncoder.encode(cursor, "UTF-8")}"; val connection = URL(endpoint).openConnection() as HttpURLConnection
                    connection.connectTimeout = 2500; connection.readTimeout = 2500
                    val code = connection.responseCode
                    if (code == 200) {
                        val json = JSONObject(connection.inputStream.bufferedReader().use { it.readText() }); val events = json.getJSONArray("events")
                        (getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager).notify(810, statusNotification("AgentBell 已连接", base))
                        prefs.edit().putLong("last_success_ms", System.currentTimeMillis()).putString("last_error", "").apply()
                        if (events.length() > 0) log("received ${events.length()} event(s), cursor=$cursor")
                        for (i in 0 until events.length()) postEvent(events.getJSONObject(i))
                        if (!json.isNull("cursor")) prefs.edit().putString("cursor", json.getString("cursor")).apply()
                    } else {
                        val error = "poll HTTP $code"
                        prefs.edit().putString("last_error", error).apply(); log(error)
                    }
                    connection.disconnect()
                }
            } catch (error: Exception) { getSharedPreferences("agentbell", Context.MODE_PRIVATE).edit().putString("last_error", error.toString()).apply(); log("poll failed: $error") }
            finally { if (wakeLock.isHeld) wakeLock.release() }
            try { Thread.sleep(3000) } catch (_: InterruptedException) { break }
        }
        stopSelf()
    }

    private fun postEvent(event: JSONObject) {
        val agent = event.optString("agent", "Agent"); val title = event.optString("title", "任务已完成"); val message = event.optString("message", "本轮任务已完成")
        val open = packageManager.getLaunchIntentForPackage(packageName); val pending = PendingIntent.getActivity(this, 0, open, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
        val notification = Notification.Builder(this, "agentbell_events").setSmallIcon(android.R.drawable.ic_dialog_info).setContentTitle("$agent · $title").setContentText(message).setStyle(Notification.BigTextStyle().bigText(message)).setAutoCancel(true).setContentIntent(pending).build()
        (getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager).notify(event.optString("id").hashCode(), notification)
        log("notification posted: ${event.optString("id")} $agent")
    }

    private fun statusNotification(title: String, text: String): Notification = Notification.Builder(this, "agentbell_status").setSmallIcon(android.R.drawable.ic_dialog_info).setContentTitle(title).setContentText(text).setOngoing(true).build()

    private fun log(message: String) {
        Log.i("AgentBell", message)
        try { filesDir.resolve("agentbell-mobile.log").appendText("${System.currentTimeMillis()} $message\n") } catch (_: Exception) { }
    }

    private fun scheduleRestart() {
        if (!getSharedPreferences("agentbell", Context.MODE_PRIVATE).getBoolean("enabled", false)) return
        try {
            val intent = Intent(this, BootReceiver::class.java).setAction("com.agentbell.mobile.RESTART")
            val pending = PendingIntent.getBroadcast(this, 811, intent, PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE)
            (getSystemService(Context.ALARM_SERVICE) as AlarmManager).setAndAllowWhileIdle(AlarmManager.ELAPSED_REALTIME_WAKEUP, SystemClock.elapsedRealtime() + 10_000, pending)
            log("service restart scheduled")
        } catch (error: Exception) { log("restart scheduling failed: $error") }
    }

    private fun createChannels() { if (Build.VERSION.SDK_INT >= 26) { val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager; manager.createNotificationChannel(NotificationChannel("agentbell_status", "连接状态", NotificationManager.IMPORTANCE_LOW)); manager.createNotificationChannel(NotificationChannel("agentbell_events", "Agent 完成通知", NotificationManager.IMPORTANCE_HIGH).apply { description = "Agent 任务完成消息" }) } }
}
