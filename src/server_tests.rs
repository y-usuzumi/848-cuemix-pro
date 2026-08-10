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
    assert!(json.contains("\"channels\":[0,0,0,1"));
    assert!(json.contains("\"main_host_11_12\":5"));
    assert!(json.contains("\"headphone_host_11_12\":5"));
    assert!(json.contains("\"main_line_in_5_6\":10"));
}

#[test]
fn formats_meter_snapshots_as_server_sent_events() {
    let snapshot = MixerMeters {
        records: vec![MixerMeterRecord {
            property_id: 0x138c,
            index: 0,
            values: vec![0x6a6f],
        }],
        updated_at: Some(Instant::now()),
        error: None,
    };
    let mut event = Vec::new();
    write_mixer_meter_event(&mut event, 42, &snapshot).unwrap();
    let event = String::from_utf8(event).unwrap();
    assert!(event.starts_with("id: 42\nevent: meters\ndata: {"));
    assert!(event.contains("\"channels\":[106,111]"));
    assert!(event.ends_with("\n\n"));

    let mut headers = Vec::new();
    write_mixer_meter_event_headers(&mut headers).unwrap();
    let headers = String::from_utf8(headers).unwrap();
    assert!(headers.contains("Content-Type: text/event-stream"));
    assert!(headers.ends_with("retry: 100\n\n"));
}

#[test]
fn formats_discovered_headphone_outputs_with_infinity_and_without_negative_zero() {
    let json = headphone_outputs_json(&[
        HeadphoneOutput {
            channel_indices: [0, 1],
            attenuation: [0, 0],
            meter_path: None,
        },
        HeadphoneOutput {
            channel_indices: [2, 3],
            attenuation: [100, 13],
            meter_path: Some(MeterPath {
                property_id: 0x13ad,
                record_index: 2,
                channel_index: 0,
            }),
        },
    ]);
    assert!(json.contains("\"number\":1"));
    assert!(json.contains("\"channel_indices\":[2,3]"));
    assert!(json.contains("\"trim_db\":[0,0]"));
    assert!(json.contains("\"trim_db\":[null,-13]"));
    assert!(json.contains(
        "\"meter_path\":{\"property_id\":\"13ad\",\"record_index\":2,\"channel_index\":0}"
    ));
    assert!(!json.contains("-0"));
}

#[test]
fn formats_discovered_line_inputs_as_physical_inputs_five_and_up() {
    let json = line_inputs_json(&[
        LineInput {
            channel_index: 0,
            gain_db: 20,
            phase_inverted: false,
        },
        LineInput {
            channel_index: 7,
            gain_db: 3,
            phase_inverted: true,
        },
    ]);
    assert!(json.contains("\"number\":5,\"channel_index\":0,\"gain_db\":20,\"phase\":false"));
    assert!(json.contains("\"number\":12,\"channel_index\":7,\"gain_db\":3,\"phase\":true"));
}

#[test]
fn formats_the_combined_physical_output_inventory() {
    let json = output_inventory_json(&OutputInventory {
        line_outputs: vec![
            LineOutput {
                channel_index: 0,
                attenuation: 35,
                meter_path: Some(MeterPath {
                    property_id: 0x13ad,
                    record_index: 1,
                    channel_index: 0,
                }),
            },
            LineOutput {
                channel_index: 3,
                attenuation: 42,
                meter_path: None,
            },
        ],
        headphone_outputs: vec![HeadphoneOutput {
            channel_indices: [0, 1],
            attenuation: [100, 100],
            meter_path: None,
        }],
    });
    assert!(json.contains("\"line_outputs\":["));
    assert!(json.contains("\"channel_index\":0,\"attenuation\":35,\"trim_db\":-35"));
    assert!(json.contains(
        "\"meter_path\":{\"property_id\":\"13ad\",\"record_index\":1,\"channel_index\":0}"
    ));
    assert!(json.contains("\"channel_index\":3,\"attenuation\":42,\"trim_db\":-42"));
    assert!(json.contains("\"headphone_outputs\":["));
    assert!(json.contains("\"trim_db\":[null,null]"));
}
