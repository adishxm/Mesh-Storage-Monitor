#![allow(clippy::missing_safety_doc)]

use jni::JNIEnv;
use jni::objects::{JClass, JString};
use jni::sys::{jboolean, jdouble, jint, jlong, jstring};
use mesh_core::{
    AndroidDeviceState, AndroidNetworkPolicy, AndroidPowerPolicy, evaluate_android_policy,
};
use mesh_node::identity::load_or_create_keypair;
use mesh_node::state::NodeState;
use serde::Serialize;
use std::ffi::{CStr, CString};
use std::os::raw::c_char;
use std::path::PathBuf;

pub struct AndroidNodeHandle {
    pub state: NodeState,
}

#[derive(Serialize)]
pub struct AndroidNodeStatusReport {
    pub peer_id: String,
    pub state: String,
    pub storage_used: u64,
    pub storage_quota: u64,
    pub usage_ratio: f64,
    pub trusted_peer_count: usize,
}

// -----------------------------------------------------------------------------
// Pure C-ABI FFI Layer
// -----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_android_init_node(
    data_dir_ptr: *const c_char,
    quota_gb: f64,
) -> *mut AndroidNodeHandle {
    if data_dir_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let c_str = unsafe { CStr::from_ptr(data_dir_ptr) };
    let path_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };
    let data_dir = PathBuf::from(path_str);

    let keypair = match load_or_create_keypair(&data_dir) {
        Ok(k) => k,
        Err(_) => return std::ptr::null_mut(),
    };
    let peer_id = libp2p::PeerId::from(keypair.public());
    let state = NodeState::with_data_dir(peer_id, data_dir, quota_gb);

    Box::into_raw(Box::new(AndroidNodeHandle { state }))
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_android_get_status(
    handle_ptr: *mut AndroidNodeHandle,
) -> *mut c_char {
    if handle_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let handle = unsafe { &*handle_ptr };
    let report = AndroidNodeStatusReport {
        peer_id: handle.state.peer_id.to_string(),
        state: format!("{:?}", handle.state.state),
        storage_used: handle.state.storage_used,
        storage_quota: handle.state.storage_quota,
        usage_ratio: handle.state.quota_tracker.usage_ratio(),
        trusted_peer_count: handle.state.trusted_peers.len(),
    };

    let json_bytes = match serde_json::to_string(&report) {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    match CString::new(json_bytes) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_android_pause(handle_ptr: *mut AndroidNodeHandle) -> i32 {
    if handle_ptr.is_null() {
        return -1;
    }
    let handle = unsafe { &mut *handle_ptr };
    match handle.state.pause() {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_android_resume(handle_ptr: *mut AndroidNodeHandle) -> i32 {
    if handle_ptr.is_null() {
        return -1;
    }
    let handle = unsafe { &mut *handle_ptr };
    match handle.state.resume() {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_android_leave(handle_ptr: *mut AndroidNodeHandle) -> i32 {
    if handle_ptr.is_null() {
        return -1;
    }
    let handle = unsafe { &mut *handle_ptr };
    match handle.state.leave() {
        Ok(_) => 0,
        Err(_) => 1,
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_android_evaluate_policy(
    state_json_ptr: *const c_char,
    only_charging: bool,
    unmetered_wifi_only: bool,
) -> *mut c_char {
    if state_json_ptr.is_null() {
        return std::ptr::null_mut();
    }
    let c_str = unsafe { CStr::from_ptr(state_json_ptr) };
    let json_str = match c_str.to_str() {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let device_state: AndroidDeviceState = match serde_json::from_str(json_str) {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    let power_policy = if only_charging {
        AndroidPowerPolicy::OnlyCharging
    } else {
        AndroidPowerPolicy::AnyPower
    };

    let net_policy = if unmetered_wifi_only {
        AndroidNetworkPolicy::UnmeteredWifiOnly
    } else {
        AndroidNetworkPolicy::AnyNetwork
    };

    let decision = evaluate_android_policy(&device_state, &power_policy, &net_policy);
    let decision_json = match serde_json::to_string(&decision) {
        Ok(s) => s,
        Err(_) => return std::ptr::null_mut(),
    };

    match CString::new(decision_json) {
        Ok(cs) => cs.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_android_free_string(s: *mut c_char) {
    if !s.is_null() {
        let _ = unsafe { CString::from_raw(s) };
    }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn mesh_android_destroy(handle_ptr: *mut AndroidNodeHandle) {
    if !handle_ptr.is_null() {
        let _ = unsafe { Box::from_raw(handle_ptr) };
    }
}

// -----------------------------------------------------------------------------
// JNI Android Binding Layer (io.meshstorage.node.MeshNodeBridge)
// -----------------------------------------------------------------------------

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_io_meshstorage_node_MeshNodeBridge_nativeInitNode(
    mut env: JNIEnv,
    _class: JClass,
    data_dir: JString,
    quota_gb: jdouble,
) -> jlong {
    let dir_str: String = match env.get_string(&data_dir) {
        Ok(s) => s.into(),
        Err(_) => return 0,
    };
    let c_dir = match CString::new(dir_str) {
        Ok(c) => c,
        Err(_) => return 0,
    };
    let ptr = unsafe { mesh_android_init_node(c_dir.as_ptr(), quota_gb) };
    ptr as jlong
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_io_meshstorage_node_MeshNodeBridge_nativeGetStatus(
    env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jstring {
    let ptr = handle as *mut AndroidNodeHandle;
    let c_status = unsafe { mesh_android_get_status(ptr) };
    if c_status.is_null() {
        return std::ptr::null_mut();
    }
    let status_str = unsafe { CStr::from_ptr(c_status) }.to_string_lossy();
    let result = env
        .new_string(status_str)
        .map(|js| js.into_raw())
        .unwrap_or(std::ptr::null_mut());
    unsafe { mesh_android_free_string(c_status) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_io_meshstorage_node_MeshNodeBridge_nativePauseNode(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jint {
    let ptr = handle as *mut AndroidNodeHandle;
    unsafe { mesh_android_pause(ptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_io_meshstorage_node_MeshNodeBridge_nativeResumeNode(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jint {
    let ptr = handle as *mut AndroidNodeHandle;
    unsafe { mesh_android_resume(ptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_io_meshstorage_node_MeshNodeBridge_nativeLeaveNode(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jint {
    let ptr = handle as *mut AndroidNodeHandle;
    unsafe { mesh_android_leave(ptr) }
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_io_meshstorage_node_MeshNodeBridge_nativeEvaluatePolicy(
    mut env: JNIEnv,
    _class: JClass,
    state_json: JString,
    only_charging: jboolean,
    unmetered_wifi_only: jboolean,
) -> jstring {
    let json_str: String = match env.get_string(&state_json) {
        Ok(s) => s.into(),
        Err(_) => return std::ptr::null_mut(),
    };
    let c_json = match CString::new(json_str) {
        Ok(c) => c,
        Err(_) => return std::ptr::null_mut(),
    };
    let c_decision = unsafe {
        mesh_android_evaluate_policy(
            c_json.as_ptr(),
            only_charging != 0,
            unmetered_wifi_only != 0,
        )
    };
    if c_decision.is_null() {
        return std::ptr::null_mut();
    }
    let dec_str = unsafe { CStr::from_ptr(c_decision) }.to_string_lossy();
    let result = env
        .new_string(dec_str)
        .map(|js| js.into_raw())
        .unwrap_or(std::ptr::null_mut());
    unsafe { mesh_android_free_string(c_decision) };
    result
}

#[unsafe(no_mangle)]
pub unsafe extern "C" fn Java_io_meshstorage_node_MeshNodeBridge_nativeDestroyNode(
    _env: JNIEnv,
    _class: JClass,
    handle: jlong,
) -> jint {
    let ptr = handle as *mut AndroidNodeHandle;
    unsafe { mesh_android_destroy(ptr) };
    0
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn test_c_abi_lifecycle_and_status() {
        let temp = tempdir().unwrap();
        let path = CString::new(temp.path().to_str().unwrap()).unwrap();

        // 1. Initialize node via FFI
        let handle = unsafe { mesh_android_init_node(path.as_ptr(), 0.5) };
        assert!(!handle.is_null());

        // 2. Fetch status JSON
        let status_ptr = unsafe { mesh_android_get_status(handle) };
        assert!(!status_ptr.is_null());
        let status_str = unsafe { CStr::from_ptr(status_ptr) }.to_str().unwrap();
        assert!(status_str.contains("Active"));
        unsafe { mesh_android_free_string(status_ptr) };

        // 3. Pause
        assert_eq!(unsafe { mesh_android_pause(handle) }, 0);
        let status_ptr2 = unsafe { mesh_android_get_status(handle) };
        let status_str2 = unsafe { CStr::from_ptr(status_ptr2) }.to_str().unwrap();
        assert!(status_str2.contains("Paused"));
        unsafe { mesh_android_free_string(status_ptr2) };

        // 4. Resume
        assert_eq!(unsafe { mesh_android_resume(handle) }, 0);

        // 5. Destroy
        unsafe { mesh_android_destroy(handle) };
    }

    #[test]
    fn test_c_abi_evaluate_policy() {
        let state = AndroidDeviceState {
            total_device_storage_bytes: 128_000_000_000,
            free_device_storage_bytes: 60_000_000_000,
            is_charging: true,
            is_unmetered_wifi: true,
            requested_contribution_pct: 2.0,
        };
        let state_json = CString::new(serde_json::to_string(&state).unwrap()).unwrap();

        let decision_ptr = unsafe { mesh_android_evaluate_policy(state_json.as_ptr(), true, true) };
        assert!(!decision_ptr.is_null());
        let decision_str = unsafe { CStr::from_ptr(decision_ptr) }.to_str().unwrap();
        assert!(decision_str.contains("\"allowed_to_operate\":true"));
        unsafe { mesh_android_free_string(decision_ptr) };
    }
}
