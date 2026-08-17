package com.agentbell.mobile

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import androidx.core.content.ContextCompat

class BootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val allowedAction = intent.action == Intent.ACTION_BOOT_COMPLETED || intent.action == Intent.ACTION_MY_PACKAGE_REPLACED || intent.action == "com.agentbell.mobile.RESTART"
        if (allowedAction && context.getSharedPreferences("agentbell", Context.MODE_PRIVATE).getBoolean("enabled", false)) ContextCompat.startForegroundService(context, Intent(context, AgentBellService::class.java))
    }
}
