use super::*;

#[test]
fn authorizes_only_the_local_page_with_its_session_token() {
    let token = "0123456789abcdef";
    assert!(is_authorized(
        Some("http://127.0.0.1:8480"),
        Some(&token.to_string()),
        "http://127.0.0.1:8480",
        token
    ));
    assert!(!is_authorized(
        Some("https://example.test"),
        Some(&token.to_string()),
        "http://127.0.0.1:8480",
        token
    ));
    assert!(!is_authorized(
        Some("http://127.0.0.1:8480"),
        Some(&"wrong".to_string()),
        "http://127.0.0.1:8480",
        token
    ));
}

#[test]
fn generates_a_hex_session_token() {
    let token = new_session_token().expect("session token");
    assert_eq!(token.len(), 64);
    assert!(token
        .bytes()
        .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte)));
}

#[test]
fn limits_proxying_to_the_configured_device() {
    let scope = ServerScope::Configured("192.168.4.166".to_string());
    let params = parse_query("host=192.168.4.166");
    assert_eq!(
        allowed_host(&params, &scope),
        Ok("192.168.4.166".to_string())
    );
    let params = parse_query("host=192.168.4.1");
    assert!(allowed_host(&params, &scope).is_err());
}

#[test]
fn hostless_server_allows_only_discovered_control_addresses() {
    let scope = ServerScope::Discovered(vec![DiscoveryResult {
        instance: "848._avdecc._tcp.local".to_string(),
        host: "848.local".to_string(),
        port: 17221,
        addresses: vec!["192.168.4.166".to_string(), "fe80::1".to_string()],
        txt: Vec::new(),
    }]);
    let params = parse_query("host=192.168.4.166");
    assert_eq!(
        allowed_host(&params, &scope),
        Ok("192.168.4.166".to_string())
    );
    assert!(allowed_host(&HashMap::new(), &scope).is_err());
    let params = parse_query("host=192.168.4.1");
    assert!(allowed_host(&params, &scope).is_err());
}

#[test]
fn decodes_form_queries() {
    let params = parse_query("path=%2Fdatastore%2Fext&value=Main+out");
    assert_eq!(params["path"], "/datastore/ext");
    assert_eq!(params["value"], "Main out");
}

#[test]
fn formats_the_operating_system_assigned_loopback_port() {
    let origin = origin_for_address("127.0.0.1:43123".parse().unwrap());
    assert_ne!(origin, "http://127.0.0.1:0");
    assert!(origin.starts_with("http://127.0.0.1:"));
}

#[test]
fn enforces_a_total_request_deadline() {
    let started = Instant::now()
        .checked_sub(Duration::from_millis(2))
        .unwrap();
    assert!(check_request_deadline(started, Duration::from_millis(1)).is_err());
}

#[test]
fn reads_the_entity_id_from_a_datastore_snapshot() {
    assert_eq!(
        datastore_entity_id("{\"uid\":\"0001f2fffefeb9e2\"}"),
        Ok(0x0001_f2ff_fefe_b9e2)
    );
    assert!(datastore_entity_id("{\"uid\":\"not-an-entity-id\"}").is_err());
}

#[test]
fn formats_all_raw_meter_records_and_the_validated_fader_pairs() {
    let snapshot = MixerMeters {
        records: vec![MixerMeterRecord {
            property_id: 0x13ad,
            index: 0,
            values: (0..22).collect(),
        }],
        updated_at: Some(Instant::now()),
        error: None,
    };
    let json = mixer_meters_json(&snapshot);
    assert!(json.contains("\"property_id\":\"13ad\""));
    assert!(json.contains("\"main_host_11_12\":5"));
    assert!(json.contains("\"headphone_host_11_12\":5"));
    assert!(json.contains("\"main_line_in_5_6\":10"));
}
