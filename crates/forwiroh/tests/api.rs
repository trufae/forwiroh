use forwiroh::{ErrorCode, Forwiroh, Options, TransportMode};
use serde_json::json;

#[tokio::test]
async fn supported_facade_opens_commands_and_closes_without_transport_types() {
    let path = tempfile::tempdir().unwrap();
    let core = Forwiroh::open_with_options(
        path.path(),
        "Rust consumer",
        Options::default().transport(TransportMode::LocalOnly),
    )
    .await
    .unwrap();
    let status = core.command(json!({"op":"status"})).await.unwrap();
    assert_eq!(status["id"], core.id());
    assert_eq!(status["name"], "Rust consumer");
    let info = core.api_info().await.unwrap();
    assert_eq!(info.control_api_version, forwiroh::CONTROL_API_VERSION);
    assert_eq!(info.c_abi_version, forwiroh::C_ABI_VERSION);
    assert_eq!(info.max_packet_size, forwiroh::MAX_PACKET_SIZE);
    for command in ["forward", "forward_start", "pair", "pair_start", "stop"] {
        assert!(info.supports(command));
    }
    assert!(!info.supports("unknown"));
    assert!(info.backends.iter().any(|b| b == "vpn_packet"));
    assert_eq!(
        core.command(json!({"op":"stop", "id":"missing"}))
            .await
            .unwrap(),
        serde_json::Value::Null
    );
    assert_eq!(
        core.command(json!({"op":"not_a_command"}))
            .await
            .unwrap_err()
            .code(),
        ErrorCode::InvalidCommand
    );
    assert_eq!(
        core.receive_packet("missing").await.unwrap_err().code(),
        ErrorCode::SessionNotFound
    );
    core.close().await;
    core.close().await;
    assert!(core.next_event().await.is_none());
    assert_eq!(
        core.command(json!({"op":"status"}))
            .await
            .unwrap_err()
            .code(),
        ErrorCode::CoreStopped
    );
}

#[tokio::test]
async fn dropping_the_facade_cancels_its_background_ownership() {
    let path = tempfile::tempdir().unwrap();
    let options = Options::default().transport(TransportMode::LocalOnly);
    let core = Forwiroh::open_with_options(path.path(), "owner", options.clone())
        .await
        .unwrap();
    drop(core);
    tokio::time::timeout(std::time::Duration::from_secs(5), async {
        loop {
            match Forwiroh::open_with_options(path.path(), "next", options.clone()).await {
                Ok(core) => {
                    core.close().await;
                    break;
                }
                Err(error) => {
                    assert!(
                        error.to_string().contains("state is already in use"),
                        "{error}"
                    );
                    tokio::time::sleep(std::time::Duration::from_millis(10)).await;
                }
            }
        }
    })
    .await
    .unwrap();
}
