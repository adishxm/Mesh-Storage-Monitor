package io.meshstorage.node

/**
 * JNI wrapper communicating with `libmesh_android_bridge.so`.
 */
object MeshNodeBridge {
    init {
        try {
            System.loadLibrary("mesh_android_bridge")
        } catch (e: UnsatisfiedLinkError) {
            // In unit tests or mock environments where native library is not packed
            System.err.println("MeshNodeBridge: Native library not loaded in current environment: ${e.message}")
        }
    }

    external fun nativeInitNode(dataDir: String, quotaGb: Double): Long
    external fun nativeGetStatus(handle: Long): String?
    external fun nativePauseNode(handle: Long): Int
    external fun nativeResumeNode(handle: Long): Int
    external fun nativeLeaveNode(handle: Long): Int
    external fun nativeEvaluatePolicy(
        stateJson: String,
        onlyCharging: Boolean,
        unmeteredWifiOnly: Boolean
    ): String?
    external fun nativeDestroyNode(handle: Long): Int
}

data class AndroidNodeStatus(
    val peerId: String,
    val state: String,
    val storageUsed: Long,
    val storageQuota: Long,
    val usageRatio: Double,
    val trustedPeerCount: Int
)

data class AndroidPolicyResult(
    val allowedToOperate: Boolean,
    val effectiveQuotaBytes: Long,
    val reason: String
)
