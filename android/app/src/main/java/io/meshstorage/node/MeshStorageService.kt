package io.meshstorage.node

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.Service
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.net.ConnectivityManager
import android.net.Network
import android.net.NetworkCapabilities
import android.net.NetworkRequest
import android.os.BatteryManager
import android.os.Build
import android.os.IBinder
import androidx.core.app.NotificationCompat
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.Job
import kotlinx.coroutines.delay
import kotlinx.coroutines.isActive
import kotlinx.coroutines.launch
import org.json.JSONObject
import java.io.File

class MeshStorageService : Service() {

    private val serviceJob = Job()
    private val serviceScope = CoroutineScope(Dispatchers.IO + serviceJob)

    private var nodeHandle: Long = 0
    private var isCharging: Boolean = false
    private var isUnmeteredWifi: Boolean = false

    private var requireCharging: Boolean = true
    private var requireWifiOnly: Boolean = true
    private var requestedQuotaPct: Double = 2.0

    private lateinit var connectivityManager: ConnectivityManager

    private val batteryReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            val status = intent?.getIntExtra(BatteryManager.EXTRA_STATUS, -1) ?: -1
            isCharging = status == BatteryManager.BATTERY_STATUS_CHARGING ||
                    status == BatteryManager.BATTERY_STATUS_FULL
            checkPolicyAndApply()
        }
    }

    private val networkCallback = object : ConnectivityManager.NetworkCallback() {
        override fun onAvailable(network: Network) {
            checkNetworkCapabilities()
        }

        override fun onLost(network: Network) {
            isUnmeteredWifi = false
            checkPolicyAndApply()
        }

        override fun onCapabilitiesChanged(network: Network, caps: NetworkCapabilities) {
            checkNetworkCapabilities()
        }
    }

    override fun onCreate() {
        super.onCreate()
        connectivityManager = getSystemService(Context.CONNECTIVITY_SERVICE) as ConnectivityManager
        createNotificationChannel()

        val notification = buildNotification("Initializing storage node...", 0, 0)
        startForeground(NOTIFICATION_ID, notification)

        // Setup native node in app internal storage sandbox
        val dataDir = File(filesDir, "mesh_node_data").apply { mkdirs() }
        nodeHandle = MeshNodeBridge.nativeInitNode(dataDir.absolutePath, 1.0)

        // Register power receiver
        registerReceiver(batteryReceiver, IntentFilter(Intent.ACTION_BATTERY_CHANGED))

        // Register network callback
        val request = NetworkRequest.Builder()
            .addCapability(NetworkCapabilities.NET_CAPABILITY_INTERNET)
            .build()
        connectivityManager.registerNetworkCallback(request, networkCallback)

        // Start periodic telemetry loop
        startTelemetryLoop()
    }

    private fun checkNetworkCapabilities() {
        val activeNet = connectivityManager.activeNetwork
        val caps = connectivityManager.getNetworkCapabilities(activeNet)
        isUnmeteredWifi = caps?.let {
            it.hasTransport(NetworkCapabilities.TRANSPORT_WIFI) &&
                    it.hasCapability(NetworkCapabilities.NET_CAPABILITY_NOT_METERED)
        } ?: false
        checkPolicyAndApply()
    }

    private fun checkPolicyAndApply() {
        if (nodeHandle == 0L) return

        val internalStorage = filesDir
        val totalStorage = internalStorage.totalSpace
        val freeStorage = internalStorage.freeSpace

        val stateJson = JSONObject().apply {
            put("total_device_storage_bytes", totalStorage)
            put("free_device_storage_bytes", freeStorage)
            put("is_charging", isCharging)
            put("is_unmetered_wifi", isUnmeteredWifi)
            put("requested_contribution_pct", requestedQuotaPct)
        }.toString()

        val decisionRaw = MeshNodeBridge.nativeEvaluatePolicy(stateJson, requireCharging, requireWifiOnly)
        if (decisionRaw != null) {
            try {
                val decision = JSONObject(decisionRaw)
                val allowed = decision.optBoolean("allowed_to_operate", false)
                val reason = decision.optString("reason", "")

                if (allowed) {
                    MeshNodeBridge.nativeResumeNode(nodeHandle)
                    updateNotification("Active: $reason")
                } else {
                    MeshNodeBridge.nativePauseNode(nodeHandle)
                    updateNotification("Paused: $reason")
                }
            } catch (e: Exception) {
                e.printStackTrace()
            }
        }
    }

    private fun startTelemetryLoop() {
        serviceScope.launch {
            while (isActive) {
                delay(5000)
                if (nodeHandle != 0L) {
                    val statusJson = MeshNodeBridge.nativeGetStatus(nodeHandle)
                    if (statusJson != null) {
                        try {
                            val status = JSONObject(statusJson)
                            val state = status.optString("state", "Unknown")
                            val used = status.optLong("storage_used", 0)
                            val quota = status.optLong("storage_quota", 0)
                            updateNotification("Status: $state | Used: ${used / 1024} KB / ${quota / (1024 * 1024)} MB")
                        } catch (e: Exception) {
                            e.printStackTrace()
                        }
                    }
                }
            }
        }
    }

    private fun updateNotification(text: String) {
        val notification = buildNotification(text, 0, 0)
        val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
        manager.notify(NOTIFICATION_ID, notification)
    }

    private fun buildNotification(text: String, usedBytes: Long, quotaBytes: Long): Notification {
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("Mesh Storage Cloud Node")
            .setContentText(text)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_LOW)
            .build()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val channel = NotificationChannel(
                CHANNEL_ID,
                "Mesh Storage Background Sync",
                NotificationManager.IMPORTANCE_LOW
            ).apply {
                description = "Shows live peer-to-peer storage contribution and network status"
            }
            val manager = getSystemService(Context.NOTIFICATION_SERVICE) as NotificationManager
            manager.createNotificationChannel(channel)
        }
    }

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onDestroy() {
        super.onDestroy()
        serviceJob.cancel()
        try {
            unregisterReceiver(batteryReceiver)
            connectivityManager.unregisterNetworkCallback(networkCallback)
        } catch (_: Exception) {}

        if (nodeHandle != 0L) {
            MeshNodeBridge.nativeDestroyNode(nodeHandle)
            nodeHandle = 0L
        }
    }

    companion object {
        const val CHANNEL_ID = "mesh_storage_foreground_channel"
        const val NOTIFICATION_ID = 4001
    }
}
