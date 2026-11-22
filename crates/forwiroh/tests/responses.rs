use forwiroh::{Event, Invitation, PairingRequest, PeerPolicy, Status, VpnSession};
use serde_json::Value;

#[test]
fn successful_response_fixtures_are_tolerant_and_keep_policy_extensions() {
    let fixtures: Value =
        serde_json::from_str(include_str!("../../../Tests/Fixtures/control-success.json")).unwrap();
    for key in ["status_legacy", "status_current"] {
        let status: Status = serde_json::from_value(fixtures[key].clone()).unwrap();
        assert_eq!(status.id, "node");
    }
    let invitation: Invitation = serde_json::from_value(fixtures["invitation"].clone()).unwrap();
    assert_eq!(invitation.id, "invitation");
    let pending: Vec<PairingRequest> = serde_json::from_value(fixtures["pending"].clone()).unwrap();
    assert_eq!(pending[0].id, "request");
    for key in ["vpn_legacy", "vpn_current"] {
        let session: VpnSession = serde_json::from_value(fixtures[key].clone()).unwrap();
        assert_eq!(session.session_id.as_str(), "session");
    }
    let event: Event = serde_json::from_value(fixtures["event"].clone()).unwrap();
    assert_eq!(event.event, "future_event");
    assert_eq!(event.fields["opaque"]["values"][1], true);
    let mut policy: PeerPolicy = serde_json::from_value(fixtures["policy_future"].clone()).unwrap();
    policy.restricted = false;
    let encoded = serde_json::to_value(policy).unwrap();
    assert_eq!(
        encoded["future_permission"],
        fixtures["policy_future"]["future_permission"]
    );
    assert_eq!(encoded["vpn"]["future_permission"], false);
    let legacy: PeerPolicy = serde_json::from_value(fixtures["policy_legacy"].clone()).unwrap();
    assert!(!legacy.restricted);
    assert!(legacy.socks_ports.is_empty());
    // Adding backend policy fields requires an explicit schema/version review.
    assert_eq!(forwiroh::POLICY_SCHEMA_VERSION, 1);
    let backend = serde_json::to_value(forwiroh::store::Policy::default()).unwrap();
    assert_eq!(
        serde_json::to_value(backend.as_object().unwrap().keys().collect::<Vec<_>>()).unwrap(),
        fixtures["policy_schema_v1_fields"]
    );
    let vpn: forwiroh::store::VpnPolicy = serde_json::from_value(serde_json::json!({
        "routes":["198.18.0.0/16"],"local_ip":"10.250.0.1","peer_ip":"10.250.0.2"
    }))
    .unwrap();
    let backend = serde_json::to_value(vpn).unwrap();
    assert_eq!(
        serde_json::to_value(backend.as_object().unwrap().keys().collect::<Vec<_>>()).unwrap(),
        fixtures["vpn_policy_schema_v1_fields"]
    );
}
