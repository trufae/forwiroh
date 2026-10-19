//! Small C ABI: commands/events are JSON; VPN packets cross the boundary as bytes.
//! Calls may run concurrently. Shutdown wakes blocked calls; free only after joining
//! all callers. Never call these blocking functions on the UI thread.
use crate::{Node, control::Command, vpn::VpnLink};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    ffi::{CStr, CString, c_char},
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::{
    runtime::Runtime,
    sync::{Mutex as AsyncMutex, broadcast, mpsc},
};

type PacketReader = Arc<AsyncMutex<mpsc::Receiver<Vec<u8>>>>;
/// Returns a static, NUL-terminated version string. Do not free it.
#[unsafe(no_mangle)]
pub extern "C" fn forwiroh_version() -> *const c_char {
    concat!(env!("CARGO_PKG_VERSION"), "\0").as_ptr().cast()
}

/// Returns public crypto capabilities as owned JSON; release with string_free.
#[unsafe(no_mangle)]
pub extern "C" fn forwiroh_crypto_info() -> *mut c_char {
    string(crate::crypto_info())
}

/// Returns a QR module matrix as JSON, without requiring a running endpoint.
/// # Safety
/// Input must be a valid NUL-terminated string for the duration of this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_qr_code(input: *const c_char) -> *mut c_char {
    reply((|| {
        let input = unsafe { text(input)? };
        ensure!(input.len() <= 4096, "QR input too large");
        let code = qrcode::QrCode::new(input)?;
        let width = code.width();
        let pixels: Vec<bool> = code
            .into_colors()
            .into_iter()
            .map(|color| color == qrcode::Color::Dark)
            .collect();
        Ok(json!({"width":width, "pixels":pixels}))
    })())
}

pub struct Handle {
    runtime: Runtime,
    node: Arc<Node>,
    events: AsyncMutex<broadcast::Receiver<Value>>,
    packets: Mutex<HashMap<String, PacketReader>>,
}

fn open_handle(path: &str, name: &str, key: Option<[u8; 32]>) -> Result<Handle> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;
    let node = runtime.block_on(Node::open(
        Path::new(path),
        name,
        key,
        crate::TransportMode::Automatic,
    ))?;
    let events = AsyncMutex::new(node.subscribe());
    Ok(Handle {
        runtime,
        node,
        events,
        packets: Mutex::default(),
    })
}

unsafe fn opened(result: Result<Handle>, error: *mut *mut c_char) -> *mut Handle {
    if !error.is_null() {
        unsafe {
            *error = std::ptr::null_mut();
        }
    }
    match result {
        Ok(h) => Box::into_raw(Box::new(h)),
        Err(e) => {
            if !error.is_null() {
                unsafe {
                    *error = reply(Err(e));
                }
            }
            std::ptr::null_mut()
        }
    }
}

/// Opens a desktop endpoint using the persistent identity in the state directory.
/// # Safety
/// Strings must be valid C strings; error must be NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_open_desktop(
    path: *const c_char,
    name: *const c_char,
    error: *mut *mut c_char,
) -> *mut Handle {
    let result = (|| open_handle(unsafe { text(path)? }, unsafe { text(name)? }, None))();
    unsafe { opened(result, error) }
}
fn string(value: Value) -> *mut c_char {
    CString::new(value.to_string())
        .expect("JSON contains no raw NUL")
        .into_raw()
}
fn reply(result: Result<Value>) -> *mut c_char {
    string(match result {
        Ok(data) => json!({"data":data}),
        Err(e) => json!({"error":e.to_string()}),
    })
}
unsafe fn text<'a>(p: *const c_char) -> Result<&'a str> {
    ensure!(!p.is_null(), "null string");
    // SAFETY: caller guarantees a valid NUL-terminated string for the duration of the call.
    let s = unsafe { CStr::from_ptr(p) }.to_str()?;
    ensure!(s.len() <= 8192, "input too large");
    Ok(s)
}
/// # Safety
/// Strings must be valid C strings; key points to 32 bytes; error is writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_open(
    path: *const c_char,
    name: *const c_char,
    key: *const u8,
    error: *mut *mut c_char,
) -> *mut Handle {
    let result = (|| -> Result<Handle> {
        let path = unsafe { text(path)? };
        let name = unsafe { text(name)? };
        ensure!(!key.is_null(), "Keychain identity required");
        let mut bytes = [0; 32];
        bytes.copy_from_slice(unsafe { std::slice::from_raw_parts(key, 32) });
        open_handle(path, name, Some(bytes))
    })();
    unsafe { opened(result, error) }
}
/// # Safety
/// Handle remains live throughout this call; input is a valid C string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_command(
    handle: *mut Handle,
    input: *const c_char,
) -> *mut c_char {
    reply((|| {
        let h = unsafe { handle.as_ref() }.context("null handle")?;
        let input: Value = serde_json::from_str(unsafe { text(input)? })?;
        h.runtime.block_on(async {
            ensure!(!h.node.cancel.is_cancelled(),"core stopped");
            if input["op"] == "vpn_start" {
                let peer = input["peer"].as_str().context("missing peer")?;
                let routes = serde_json::from_value(input["routes"].clone())?;
                {
                    let mut readers = h.packets.lock().unwrap();
                    readers.retain(|_, reader| reader.try_lock().map_or(true, |rx| !rx.is_closed()));
                    ensure!(readers.len() < 8, "packet reader limit reached");
                }
                let VpnLink { id,local_ip,remote_ip,routes,packets } = tokio::select! {
                    result = h.node.start_vpn_with_transport(peer,routes,cfg!(target_os = "ios")) => result?,
                    _ = h.node.cancel.cancelled() => anyhow::bail!("core stopped"),
                };
                h.packets.lock().unwrap().insert(id.clone(),Arc::new(AsyncMutex::new(packets)));
                return Ok(json!({"id":id,"local_ip":local_ip,"remote_ip":remote_ip,"local_ipv6":crate::vpn::ipv6(local_ip),"remote_ipv6":crate::vpn::ipv6(remote_ip),"forward_ip":crate::vpn::FORWARD_IP,"routes":routes,"mtu":crate::vpn::MTU}));
            }
            if input["op"] == "stop_vpn" && let Some(id)=input["id"].as_str() { h.packets.lock().unwrap().remove(id); }
            let command: Command = serde_json::from_value(input)?;
            tokio::select! { r = h.node.command(command) => r, _ = h.node.cancel.cancelled() => anyhow::bail!("core stopped") }
        })
    })())
}
/// # Safety
/// Handle must remain live. Returns NULL on shutdown; release non-NULL with string_free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_next_event(handle: *mut Handle) -> *mut c_char {
    let Some(h) = (unsafe { handle.as_ref() }) else {
        return std::ptr::null_mut();
    };
    h.runtime.block_on(async {
        let mut events=h.events.lock().await;
        tokio::select! {
            _ = h.node.cancel.cancelled()=>std::ptr::null_mut(),
            e = events.recv()=>match e { Ok(v)=>string(v),Err(broadcast::error::RecvError::Lagged(_))=>string(json!({"event":"resync"})),Err(_)=>std::ptr::null_mut() }
        }
    })
}
/// # Safety
/// Handle, session C string and packet bytes must remain valid during this call.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_send_packet(
    handle: *mut Handle,
    session: *const c_char,
    bytes: *const u8,
    len: usize,
) -> i32 {
    let result = (|| -> Result<()> {
        let h = unsafe { handle.as_ref() }.context("null handle")?;
        ensure!(
            !bytes.is_null() && len <= crate::vpn::MAX_PACKET,
            "invalid packet"
        );
        h.node.send_packet(unsafe { text(session)? }, unsafe {
            std::slice::from_raw_parts(bytes, len)
        })
    })();
    if result.is_ok() { 0 } else { -1 }
}
/// # Safety
/// Buffer must hold capacity bytes and capacity must be at least MTU. Blocks until
/// a packet or shutdown. Returns packet length, 0 for closed session, -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_receive_packet(
    handle: *mut Handle,
    session: *const c_char,
    buffer: *mut u8,
    capacity: usize,
) -> i32 {
    let result = (|| -> Result<i32> {
        let h = unsafe { handle.as_ref() }.context("null handle")?;
        ensure!(
            !buffer.is_null() && capacity >= crate::vpn::MAX_PACKET,
            "buffer too small"
        );
        let id = unsafe { text(session)? };
        let reader = h
            .packets
            .lock()
            .unwrap()
            .get(id)
            .cloned()
            .context("unknown VPN session")?;
        let packet = h.runtime.block_on(async {
            let mut rx = reader.lock().await;
            tokio::select! { _ = h.node.cancel.cancelled()=>None, p = rx.recv()=>p }
        });
        match packet {
            Some(p) => {
                unsafe {
                    std::ptr::copy_nonoverlapping(p.as_ptr(), buffer, p.len());
                }
                Ok(p.len() as i32)
            }
            None => {
                h.packets.lock().unwrap().remove(id);
                Ok(0)
            }
        }
    })();
    result.unwrap_or(-1)
}
/// # Safety
/// Handle must remain live; wakes event/packet calls. May be called once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_shutdown(handle: *mut Handle) {
    if let Some(h) = unsafe { handle.as_ref() } {
        h.runtime.block_on(h.node.shutdown());
    }
}
/// # Safety
/// Call only after shutdown has returned AND every concurrent call has returned.
/// Must be called exactly once for a handle returned by open.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_free(handle: *mut Handle) {
    if !handle.is_null() {
        drop(unsafe { Box::from_raw(handle) });
    }
}
/// # Safety
/// Pointer must be a non-freed string returned by this API (NULL is allowed).
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_string_free(value: *mut c_char) {
    if !value.is_null() {
        drop(unsafe { CString::from_raw(value) });
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn qr_matrix_and_invalid_ffi_inputs() {
        let input = CString::new("forwiroh QR test").unwrap();
        unsafe {
            let raw = forwiroh_qr_code(input.as_ptr());
            let value: Value = serde_json::from_str(CStr::from_ptr(raw).to_str().unwrap()).unwrap();
            let width = value["data"]["width"].as_u64().unwrap() as usize;
            assert_eq!(
                value["data"]["pixels"].as_array().unwrap().len(),
                width * width
            );
            assert!(value["data"]["pixels"][0].as_bool().unwrap());
            forwiroh_string_free(raw);
            let raw = forwiroh_qr_code(std::ptr::null());
            assert!(
                CStr::from_ptr(raw)
                    .to_str()
                    .unwrap()
                    .contains("null string")
            );
            forwiroh_string_free(raw);
            let mut error = std::ptr::null_mut();
            assert!(forwiroh_open_desktop(std::ptr::null(), input.as_ptr(), &mut error).is_null());
            assert!(!error.is_null());
            forwiroh_string_free(error);
        }
    }
    #[test]
    fn ffi_commands_and_blocked_event_reader_shutdown_cleanly() {
        let dir = tempfile::tempdir().unwrap();
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let node = runtime
            .block_on(Node::open(
                dir.path(),
                "ffi",
                Some([42; 32]),
                crate::TransportMode::LocalOnly,
            ))
            .unwrap();
        let events = AsyncMutex::new(node.subscribe());
        let handle = Box::into_raw(Box::new(Handle {
            runtime,
            node,
            events,
            packets: Mutex::default(),
        }));
        let command = CString::new(r#"{"op":"status"}"#).unwrap();
        unsafe {
            let result = forwiroh_command(handle, command.as_ptr());
            let json: Value =
                serde_json::from_str(CStr::from_ptr(result).to_str().unwrap()).unwrap();
            assert_eq!(json["data"]["name"], "ffi");
            forwiroh_string_free(result);
            let raw = handle as usize;
            let reader =
                std::thread::spawn(move || forwiroh_next_event(raw as *mut Handle) as usize);
            forwiroh_shutdown(handle);
            assert_eq!(reader.join().unwrap(), 0);
            let result = forwiroh_command(handle, command.as_ptr());
            assert!(
                CStr::from_ptr(result)
                    .to_str()
                    .unwrap()
                    .contains("core stopped")
            );
            forwiroh_string_free(result);
            forwiroh_free(handle);
        }
    }
}

/// Install the process-lifetime Application context before constructing endpoints.
/// The C JNI bridge supplies the current environment and a live Application object.
#[cfg(target_os = "android")]
#[unsafe(no_mangle)]
pub extern "system" fn forwiroh_android_init<'local>(
    mut unowned: jni::EnvUnowned<'local>,
    context: jni::objects::JObject<'local>,
) {
    use jni::{errors::ThrowRuntimeExAndDefault, objects::JObject, refs::Global};
    static CONTEXT: Mutex<Option<Global<JObject<'static>>>> = Mutex::new(None);
    unowned
        .with_env(|env| {
            let mut stored = CONTEXT.lock().unwrap();
            if stored.is_none() {
                let global = env.new_global_ref(&context)?;
                let vm = env.get_java_vm()?;
                // SAFETY: the VM lives for the process; CONTEXT retains a global
                // Application reference for the same lifetime. Called once only.
                unsafe {
                    ndk_context::initialize_android_context(
                        vm.get_raw().cast(),
                        global.as_raw().cast(),
                    );
                }
                *stored = Some(global);
            }
            rustls_platform_verifier::android::init_with_env(env, context)
        })
        .resolve::<ThrowRuntimeExAndDefault>();
}
