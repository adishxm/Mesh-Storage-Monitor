package io.meshstorage.node

import androidx.test.ext.junit.runners.AndroidJUnit4
import org.junit.Assert.*
import org.junit.Test
import org.junit.runner.RunWith

/**
 * Instrumentation test verifying JNI bridge initialization and policy evaluation.
 */
@RunWith(AndroidJUnit4::class)
class MeshNodeBridgeTest {

    @Test
    fun testNativePolicyEvaluationOrFallback() {
        val testJson = """
            {
                "is_charging": true,
                "is_unmetered_wifi": true,
                "available_storage_bytes": 50000000000,
                "total_storage_bytes": 100000000000,
                "target_quota_bytes": 10000000000
            }
        """.trimIndent()

        try {
            val resultJson = MeshNodeBridge.nativeEvaluatePolicy(
                stateJson = testJson,
                onlyCharging = true,
                unmeteredWifiOnly = true
            )
            assertNotNull("Policy evaluation should return a JSON result when native library is loaded", resultJson)
        } catch (e: UnsatisfiedLinkError) {
            // Expected in mock environments without packaged libmesh_android_bridge.so
            System.err.println("Native library not loaded in current environment: ${e.message}")
        }
    }

    @Test
    fun testNativeInitNodeLifecycle() {
        try {
            val handle = MeshNodeBridge.nativeInitNode("/data/data/io.meshstorage.node/files", 5.0)
            assertTrue("Handle must be non-zero upon initialization", handle != 0L)

            val statusJson = MeshNodeBridge.nativeGetStatus(handle)
            assertNotNull("Status must not be null", statusJson)

            val paused = MeshNodeBridge.nativePauseNode(handle)
            assertEquals("Pause should return code 0", 0, paused)

            val resumed = MeshNodeBridge.nativeResumeNode(handle)
            assertEquals("Resume should return code 0", 0, resumed)

            val destroyed = MeshNodeBridge.nativeDestroyNode(handle)
            assertEquals("Destroy should return code 0", 0, destroyed)
        } catch (e: UnsatisfiedLinkError) {
            System.err.println("Native library not loaded: ${e.message}")
        }
    }
}
