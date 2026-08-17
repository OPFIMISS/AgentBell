package com.agentbell.mobile

import android.Manifest
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.net.wifi.WifiManager
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.net.Uri
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    private var multicastLock: WifiManager.MulticastLock? = null

    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        MethodChannel(flutterEngine.dartExecutor.binaryMessenger, "agentbell/native").setMethodCallHandler { call, result ->
            when (call.method) {
                "deviceInfo" -> result.success(mapOf("manufacturer" to Build.MANUFACTURER.uppercase(), "model" to Build.MODEL.uppercase(), "id" to Settings.Secure.getString(contentResolver, Settings.Secure.ANDROID_ID), "emulator" to isEmulator()))
                "savedConnection" -> { val p = getSharedPreferences("agentbell", Context.MODE_PRIVATE); result.success(mapOf("url" to (p.getString("url", "") ?: ""), "token" to (p.getString("token", "") ?: ""), "enabled" to p.getBoolean("enabled", false), "last_success_ms" to p.getLong("last_success_ms", 0), "last_error" to (p.getString("last_error", "") ?: ""))) }
                "notificationGranted" -> result.success(Build.VERSION.SDK_INT < 33 || ContextCompat.checkSelfPermission(this, Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED)
                "nearbyGranted" -> result.success(Build.VERSION.SDK_INT < 33 || ContextCompat.checkSelfPermission(this, Manifest.permission.NEARBY_WIFI_DEVICES) == PackageManager.PERMISSION_GRANTED)
                "batteryOptimizationIgnored" -> result.success((getSystemService(Context.POWER_SERVICE) as PowerManager).isIgnoringBatteryOptimizations(packageName))
                "requestNotifications" -> { if (Build.VERSION.SDK_INT >= 33) ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.POST_NOTIFICATIONS), 812); result.success(true) }
                "requestNearby" -> { if (Build.VERSION.SDK_INT >= 33) ActivityCompat.requestPermissions(this, arrayOf(Manifest.permission.NEARBY_WIFI_DEVICES), 813); result.success(true) }
                "requestBackgroundMode" -> {
                    try { startActivity(Intent(Settings.ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS, Uri.parse("package:$packageName"))) }
                    catch (_: Exception) { startActivity(Intent(Settings.ACTION_APPLICATION_DETAILS_SETTINGS, Uri.parse("package:$packageName"))) }
                    result.success(true)
                }
                "acquireMulticast" -> { if (multicastLock == null) multicastLock = (applicationContext.getSystemService(Context.WIFI_SERVICE) as WifiManager).createMulticastLock("AgentBellDiscovery").apply { setReferenceCounted(false); acquire() }; result.success(true) }
                "startService" -> {
                    val url = call.argument<String>("url") ?: ""; val token = call.argument<String>("token") ?: ""; val cursor = call.argument<String>("cursor") ?: ""
                    getSharedPreferences("agentbell", Context.MODE_PRIVATE).edit().putString("url", url).putString("token", token).putString("cursor", cursor).putBoolean("enabled", true).apply()
                    ContextCompat.startForegroundService(this, Intent(this, AgentBellService::class.java)); result.success(true)
                }
                "stopService" -> { getSharedPreferences("agentbell", Context.MODE_PRIVATE).edit().putBoolean("enabled", false).apply(); stopService(Intent(this, AgentBellService::class.java)); result.success(true) }
                else -> result.notImplemented()
            }
        }
    }

    private fun isEmulator(): Boolean = Build.FINGERPRINT.startsWith("generic") || Build.FINGERPRINT.contains("emulator") || Build.MODEL.contains("Emulator") || Build.MODEL.contains("Android SDK") || Build.HARDWARE.contains("goldfish") || Build.HARDWARE.contains("ranchu")
}
