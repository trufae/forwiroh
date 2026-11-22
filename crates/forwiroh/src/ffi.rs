//! Small C ABI: commands/events are JSON; VPN packets cross the boundary as bytes.
//! Calls may run concurrently. Shutdown wakes blocked calls; free only after joining
//! all callers. Never call these blocking functions on the UI thread.
use crate::{
    Node,
    control::{self, CommandError, ErrorCode},
};
use anyhow::{Context, Result, ensure};
use serde_json::{Value, json};
#[cfg(target_os = "android")]
use std::sync::Mutex;
use std::{
    ffi::{CStr, CString, c_char},
    path::Path,
};
use tokio::runtime::Runtime;
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
    core: crate::Forwiroh,
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
    Ok(Handle {
        runtime,
        core: crate::Forwiroh::from_node(node),
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
    string(control::response(result))
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
        let input =
            unsafe { text(input) }.map_err(|error| ErrorCode::InvalidCommand.context(error))?;
        let input: Value = serde_json::from_str(input)
            .map_err(|error| ErrorCode::InvalidCommand.context(error.into()))?;
        h.runtime
            .block_on(h.core.command(input))
            .map_err(Into::into)
    })())
}
/// # Safety
/// Handle must remain live. Returns NULL on shutdown; release non-NULL with string_free.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_next_event(handle: *mut Handle) -> *mut c_char {
    let Some(h) = (unsafe { handle.as_ref() }) else {
        return std::ptr::null_mut();
    };
    h.runtime
        .block_on(h.core.next_event())
        .map_or(std::ptr::null_mut(), string)
}
/// Maximum packet length and minimum receive buffer capacity, independent of MTU.
#[unsafe(no_mangle)]
pub extern "C" fn forwiroh_max_packet_size() -> usize {
    crate::vpn::MAX_PACKET
}

unsafe fn packet_reply(result: Result<i32>, error: *mut *mut c_char) -> i32 {
    if !error.is_null() {
        unsafe {
            *error = std::ptr::null_mut();
        }
    }
    match result {
        Ok(value) => value,
        Err(failure) => {
            if !error.is_null() {
                unsafe {
                    *error = reply(Err(failure));
                }
            }
            -1
        }
    }
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
    unsafe { forwiroh_send_packet_ex(handle, session, bytes, len, std::ptr::null_mut()) }
}

/// Send a packet, returning owned command-style error JSON on failure.
/// # Safety
/// Inputs must be valid for this call; error must be NULL or writable.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_send_packet_ex(
    handle: *mut Handle,
    session: *const c_char,
    bytes: *const u8,
    len: usize,
    error: *mut *mut c_char,
) -> i32 {
    let result = (|| -> Result<i32> {
        let h = unsafe { handle.as_ref() }.context("null handle")?;
        h.core.ensure_running()?;
        ensure!(
            !bytes.is_null() && len > 0 && len <= crate::vpn::MAX_PACKET,
            CommandError::new(ErrorCode::InvalidPacket, "invalid packet")
        );
        let packet = unsafe { std::slice::from_raw_parts(bytes, len) };
        h.core.send_packet_raw(unsafe { text(session)? }, packet)?;
        Ok(0)
    })();
    unsafe { packet_reply(result, error) }
}

/// # Safety
/// Buffer must hold capacity bytes; capacity must be at least MAX_PACKET (65535).
/// Blocks until a packet or shutdown. Returns length, 0 for closed, -1 on error.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_receive_packet(
    handle: *mut Handle,
    session: *const c_char,
    buffer: *mut u8,
    capacity: usize,
) -> i32 {
    unsafe { forwiroh_receive_packet_ex(handle, session, buffer, capacity, std::ptr::null_mut()) }
}

/// Receive a packet, returning owned command-style error JSON on failure.
/// # Safety
/// Inputs must be valid for this call; error must be NULL or writable. Buffer
/// requires MAX_PACKET bytes. One consumer may read each session at a time.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_receive_packet_ex(
    handle: *mut Handle,
    session: *const c_char,
    buffer: *mut u8,
    capacity: usize,
    error: *mut *mut c_char,
) -> i32 {
    let result = (|| -> Result<i32> {
        let h = unsafe { handle.as_ref() }.context("null handle")?;
        ensure!(
            !buffer.is_null() && capacity >= crate::vpn::MAX_PACKET,
            CommandError::new(ErrorCode::BufferTooSmall, "buffer too small")
        );
        let packet = h
            .runtime
            .block_on(h.core.receive_packet_raw(unsafe { text(session)? }))?;
        match packet {
            Some(packet) => {
                unsafe {
                    std::ptr::copy_nonoverlapping(packet.as_ptr(), buffer, packet.len());
                }
                Ok(packet.len() as i32)
            }
            None => Ok(0),
        }
    })();
    unsafe { packet_reply(result, error) }
}
/// # Safety
/// Handle must remain live; wakes event/packet calls. May be called once.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn forwiroh_shutdown(handle: *mut Handle) {
    if let Some(h) = unsafe { handle.as_ref() } {
        h.runtime.block_on(h.core.close());
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
    use crate::control::Command;
    use std::sync::Arc;
    use tokio::sync::{Mutex as AsyncMutex, mpsc};

    fn local_handle(path: &Path) -> Handle {
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();
        let node = runtime
            .block_on(Node::open(
                path,
                "ffi",
                None,
                crate::TransportMode::LocalOnly,
            ))
            .unwrap();
        Handle {
            runtime,
            core: crate::Forwiroh::from_node(node),
        }
    }

    fn command(handle: &mut Handle, input: Value) -> Value {
        raw_command(handle, &input.to_string())
    }

    fn raw_command(handle: &mut Handle, input: &str) -> Value {
        let input = CString::new(input).unwrap();
        unsafe {
            let raw = forwiroh_command(handle, input.as_ptr());
            let reply = serde_json::from_str(CStr::from_ptr(raw).to_str().unwrap()).unwrap();
            forwiroh_string_free(raw);
            reply
        }
    }

    #[test]
    fn packet_capacity_errors_do_not_consume_data_and_closed_is_zero() {
        let dir = tempfile::tempdir().unwrap();
        let mut handle = local_handle(dir.path());
        let (sender, receiver) = mpsc::channel(1);
        sender.try_send(vec![7; crate::vpn::MTU]).unwrap();
        handle
            .core
            .packets
            .lock()
            .unwrap()
            .insert("session".into(), Arc::new(AsyncMutex::new(receiver)));
        let id = CString::new("session").unwrap();
        let mut buffer = vec![0; forwiroh_max_packet_size()];
        let mut error = std::ptr::null_mut();
        unsafe {
            assert_eq!(
                forwiroh_receive_packet_ex(
                    &mut handle,
                    id.as_ptr(),
                    buffer.as_mut_ptr(),
                    crate::vpn::MTU,
                    &mut error
                ),
                -1
            );
            let failure: Value =
                serde_json::from_str(CStr::from_ptr(error).to_str().unwrap()).unwrap();
            assert_eq!(failure["error_code"], "buffer_too_small");
            forwiroh_string_free(error);
            assert_eq!(
                forwiroh_receive_packet_ex(
                    &mut handle,
                    id.as_ptr(),
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut error
                ),
                crate::vpn::MTU as i32
            );
            assert!(error.is_null());
            assert_eq!(&buffer[..crate::vpn::MTU], vec![7; crate::vpn::MTU]);
            drop(sender);
            assert_eq!(
                forwiroh_receive_packet_ex(
                    &mut handle,
                    id.as_ptr(),
                    buffer.as_mut_ptr(),
                    buffer.len(),
                    &mut error
                ),
                0
            );
            assert!(error.is_null());
        }
        handle.runtime.block_on(handle.core.node.shutdown());
    }

    #[test]
    fn command_errors_match_the_socket_contract_fixtures() {
        let dir = tempfile::tempdir().unwrap();
        let mut handle = local_handle(dir.path());
        let fixtures: Vec<Value> =
            serde_json::from_str(include_str!("../tests/fixtures/control-errors.json")).unwrap();
        for fixture in fixtures {
            if cfg!(target_os = "linux") && fixture["platform"] == "non_linux" {
                continue;
            }
            let reply = if let Some(raw) = fixture["raw"].as_str() {
                raw_command(&mut handle, raw)
            } else {
                command(&mut handle, fixture["request"].clone())
            };
            assert_eq!(reply["error_code"], fixture["error_code"], "{fixture}");
            assert!(
                reply["error"].is_string(),
                "legacy clients still receive error text"
            );
            assert!(reply.get("data").is_none());
        }
        handle.runtime.block_on(handle.core.node.shutdown());
    }

    #[test]
    fn explicit_tun_uses_the_same_backend_as_node_commands() {
        let dir = tempfile::tempdir().unwrap();
        let mut handle = local_handle(dir.path());
        let expected = control::response(handle.runtime.block_on(handle.core.node.command(
            Command::VpnStart {
                peer: "missing".into(),
                routes: vec!["198.18.0.0/16".parse().unwrap()],
                tun: "invalid/name".into(),
            },
        )));
        for op in ["vpn_start", "vpn"] {
            assert_eq!(
                command(
                    &mut handle,
                    json!({
                        "op": op, "peer": "missing", "routes": ["198.18.0.0/16"], "tun": "invalid/name"
                    })
                ),
                expected
            );
            assert!(handle.core.packets.lock().unwrap().is_empty());
        }
        handle.runtime.block_on(handle.core.node.shutdown());
    }

    #[test]
    fn packet_vpn_without_a_tun_registers_a_reader() {
        for op in ["vpn_start", "vpn"] {
            for tun in [None, Some("")] {
                let client_dir = tempfile::tempdir().unwrap();
                let server_dir = tempfile::tempdir().unwrap();
                let mut handle = local_handle(client_dir.path());
                let server = handle
                    .runtime
                    .block_on(Node::open(
                        server_dir.path(),
                        "exit",
                        None,
                        crate::TransportMode::LocalOnly,
                    ))
                    .unwrap();
                for (node, other) in [(&handle.core.node, &server), (&server, &handle.core.node)] {
                    node.store
                        .lock()
                        .unwrap()
                        .update(|state| {
                            state.peers.insert(
                                other.endpoint.id().to_string(),
                                crate::store::Peer {
                                    name: "other".into(),
                                    device: "test".into(),
                                    addr: other.endpoint.addr(),
                                    policy: crate::store::Policy::default(),
                                },
                            );
                        })
                        .unwrap();
                }
                let mut input = json!({
                    "op": op, "peer": server.endpoint.id().to_string(), "routes": ["198.18.0.0/16"]
                });
                if let Some(tun) = tun {
                    input["tun"] = json!(tun);
                }
                let reply = command(&mut handle, input);
                let id = reply["data"]["id"].as_str().expect("packet VPN starts");
                assert!(handle.core.packets.lock().unwrap().contains_key(id));
                assert_eq!(reply["data"]["mtu"], crate::vpn::MTU);
                assert!(reply["data"]["local_ip"].is_string());
                let status = handle.core.node.snapshot();
                let tunnel = status["tunnels"][0]["id"].as_str().unwrap();
                assert_eq!(reply["data"]["session_id"], id);
                assert_eq!(reply["data"]["tunnel_id"], tunnel);
                assert_ne!(id, tunnel);
                assert_eq!(status["tunnels"][0]["session_id"], id);
                for wrong_id in [id, tunnel] {
                    for wrong_op in ["forward_stop", "reverse_stop", "socks_stop"] {
                        let error = command(&mut handle, json!({"op": wrong_op, "id": wrong_id}));
                        assert_eq!(error["error_code"], "wrong_tunnel_kind");
                        assert!(error["error"].as_str().unwrap().contains("is vpn"));
                        assert!(handle.core.packets.lock().unwrap().contains_key(id));
                        assert!(handle.core.node.vpn_stats(id).is_object());
                    }
                }
                // Exercise both accepted VPN IDs and the legacy generic stop.
                let stop_id = if tun.is_some() { tunnel } else { id };
                let stop_op = if op == "vpn" { "stop" } else { "vpn_stop" };
                assert_eq!(
                    command(&mut handle, json!({"op": stop_op, "id": stop_id})),
                    json!({"data": null})
                );
                assert!(handle.core.packets.lock().unwrap().is_empty());
                assert!(handle.core.node.vpn_stats(id).is_null());
                assert!(
                    handle.core.node.snapshot()["tunnels"]
                        .as_array()
                        .unwrap()
                        .is_empty()
                );
                for stopped_id in [id, tunnel] {
                    assert_eq!(
                        command(&mut handle, json!({"op": stop_op, "id": stopped_id})),
                        json!({"data": null})
                    );
                }
                handle.runtime.block_on(handle.core.node.shutdown());
                handle.runtime.block_on(server.shutdown());
            }
        }
    }

    #[test]
    fn stop_commands_validate_kinds_and_are_idempotent() {
        type Stop = fn(&Node, &str) -> Result<()>;
        let stops: [(&str, Stop); 4] = [
            ("forward", Node::forward_stop),
            ("reverse", Node::reverse_stop),
            ("socks", Node::socks_stop),
            ("vpn", Node::vpn_stop),
        ];
        let dir = tempfile::tempdir().unwrap();
        let mut handle = local_handle(dir.path());
        for (kind, stop) in stops {
            for op in [format!("{kind}_stop"), "stop".into()] {
                let (id, cancelled) = handle
                    .core
                    .node
                    .add_tunnel("peer".into(), kind, "test".into())
                    .unwrap();
                for (other, other_stop) in stops {
                    if other == kind {
                        continue;
                    }
                    assert!(
                        other_stop(&handle.core.node, &id)
                            .unwrap_err()
                            .to_string()
                            .contains(&format!("is {kind}"))
                    );
                    let reply = command(
                        &mut handle,
                        json!({"op": format!("{other}_stop"), "id": id}),
                    );
                    assert_eq!(reply["error_code"], "wrong_tunnel_kind");
                    assert!(
                        reply["error"]
                            .as_str()
                            .unwrap()
                            .contains(&format!("is {kind}"))
                    );
                    assert!(!cancelled.is_cancelled());
                    assert_eq!(handle.core.node.tunnel_kind(&id).as_deref(), Some(kind));
                }
                for _ in 0..2 {
                    assert_eq!(
                        command(&mut handle, json!({"op": op, "id": id})),
                        json!({"data": null})
                    );
                }
                assert!(cancelled.is_cancelled());
                assert!(handle.core.node.tunnel_kind(&id).is_none());
                stop(&handle.core.node, &id).unwrap();
            }
            stop(&handle.core.node, "missing").unwrap();
            assert_eq!(
                command(
                    &mut handle,
                    json!({"op": format!("{kind}_stop"), "id": "missing"})
                ),
                json!({"data": null})
            );
        }
        assert_eq!(
            command(&mut handle, json!({"op": "stop", "id": "missing"})),
            json!({"data": null})
        );
        handle.runtime.block_on(handle.core.node.shutdown());
    }

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
        let handle = Box::into_raw(Box::new(Handle {
            runtime,
            core: crate::Forwiroh::from_node(node),
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
            let error: Value =
                serde_json::from_str(CStr::from_ptr(result).to_str().unwrap()).unwrap();
            assert_eq!(error["error_code"], "core_stopped");
            let h = &*handle;
            assert_eq!(
                error,
                control::response(h.runtime.block_on(h.core.node.command(Command::Status)))
            );
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
