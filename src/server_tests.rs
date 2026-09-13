use super::*;

fn home_scope(devices: Vec<DiscoveryResult>) -> ServerScope {
    ServerScope::Discovered(Mutex::new(HomeDevices::new(Ok(devices))))
}

#[test]
fn home_is_available_without_devices_or_working_discovery() {
    for result in [Ok(Vec::new()), Err("no network interface".into())] {
        let scope = ServerScope::Discovered(Mutex::new(HomeDevices::new(result)));
        let response = route_browser_request(
            "GET",
            "/",
            "",
            None,
            &scope,
            "http://127.0.0.1:8480",
            "secret",
            &MeterHub::default(),
            Duration::ZERO,
        );
        assert_eq!(response.status, 200);
        assert!(response.body.contains("Choose your device"));
        assert!(response.body.contains("Connect by IP"));
        assert!(response.body.contains("No devices found yet"));
        assert!(!response.body.contains("__SESSION_TOKEN__"));
    }
}

#[test]
fn manual_ip_addresses_are_normalized_without_accepting_urls_or_hostnames() {
    for (input, expected) in [
        (" 192.168.4.166 ", "192.168.4.166"),
        ("127.0.0.1:8080", "127.0.0.1:8080"),
        ("192.168.1.50:80", "192.168.1.50"),
        ("2001:db8::1", "[2001:db8::1]"),
        ("[2001:db8::1]", "[2001:db8::1]"),
        ("[::1]:8080", "[::1]:8080"),
        ("[::1]:80", "[::1]"),
        ("fe80::1%eth2", "[fe80::1%eth2]"),
        ("[fe80::1%12]", "[fe80::1%12]"),
    ] {
        assert_eq!(manual_device_host(input).unwrap(), expected);
    }
    for invalid in [
        "",
        "848.local",
        "https://192.168.1.2",
        "192.168.1.2/path",
        "user@192.168.1.2",
        "999.1.1.1",
        "127.0.0.1:0",
        "127.0.0.1:65536",
        "[::1]:0",
        "[fe80::1%]",
        "[fe80::1%bad/zone]",
        "192.168.1.2\r\nHost: other",
    ] {
        assert!(manual_device_host(invalid).is_err(), "{invalid}");
    }
}

#[test]
fn home_actions_require_authorization_and_cannot_expand_fixed_host_mode() {
    for scope in [
        home_scope(Vec::new()),
        ServerScope::Configured("192.0.2.1".into()),
    ] {
        for path in ["/api/connect", "/api/discover"] {
            for (origin, token) in [
                (None, "secret"),
                (Some("https://example.test"), "secret"),
                (Some("http://127.0.0.1:8480"), "wrong"),
            ] {
                let response = route_browser_request(
                    "POST",
                    path,
                    &format!("token={token}&host=192.0.2.2"),
                    origin,
                    &scope,
                    "http://127.0.0.1:8480",
                    "secret",
                    &MeterHub::default(),
                    Duration::ZERO,
                );
                assert_eq!(response.status, 403);
            }
        }
    }
    for scope in [
        home_scope(Vec::new()),
        ServerScope::Configured("192.0.2.1".into()),
    ] {
        let response = route_browser_request(
            "POST",
            "/api/connect",
            "token=secret&host=bad-host",
            Some("http://127.0.0.1:8480"),
            &scope,
            "http://127.0.0.1:8480",
            "secret",
            &MeterHub::default(),
            Duration::ZERO,
        );
        assert_eq!(response.status, 400);
    }
    let scope = ServerScope::Configured("192.0.2.1".into());
    let response = route_browser_request(
        "POST",
        "/api/connect",
        "token=secret&host=192.0.2.2",
        Some("http://127.0.0.1:8480"),
        &scope,
        "http://127.0.0.1:8480",
        "secret",
        &MeterHub::default(),
        Duration::ZERO,
    );
    assert_eq!(response.status, 400);
}

#[test]
fn connecting_a_manual_device_only_reads_and_admits_it_after_a_valid_response() {
    for (status, body, succeeds) in [
        (200, r#"{"uid":"0001f2fffefeb9e2"}"#, true),
        (200, r#"{"name":"unrelated service"}"#, false),
        (503, r#"{"uid":"0001f2fffefeb9e2"}"#, false),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let host = listener.local_addr().unwrap().to_string();
        let peer = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let mut request = Vec::new();
            while !request.ends_with(b"\r\n\r\n") {
                let mut byte = [0];
                stream.read_exact(&mut byte).unwrap();
                request.push(byte[0]);
            }
            assert!(request.starts_with(b"GET /datastore HTTP/1.1\r\n"));
            assert!(!String::from_utf8_lossy(&request).contains("Content-Length:"));
            write!(
                stream,
                "HTTP/1.1 {status} Test\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )
            .unwrap();
        });
        let scope = home_scope(Vec::new());
        let params = parse_query(&format!("host={host}"));
        assert!(allowed_host(&params, &scope).is_err());
        let response = route_browser_request(
            "POST",
            "/api/connect",
            &format!("token=secret&host={host}"),
            Some("http://127.0.0.1:8480"),
            &scope,
            "http://127.0.0.1:8480",
            "secret",
            &MeterHub::default(),
            Duration::from_secs(2),
        );
        assert_eq!(
            response.status,
            if succeeds { 200 } else { 502 },
            "{}",
            response.body
        );
        assert_eq!(allowed_host(&params, &scope).is_ok(), succeeds);
        if succeeds {
            let console = route_browser_request(
                "GET",
                &format!("/?host={host}"),
                "",
                None,
                &scope,
                "http://127.0.0.1:8480",
                "secret",
                &MeterHub::default(),
                Duration::ZERO,
            );
            assert_eq!(console.status, 200);
            assert!(console.body.contains("← Devices"));
            assert!(console.body.contains(&format!("value=\"{host}\"")));
            assert!(allowed_host(&parse_query("host=192.0.2.3"), &scope).is_err());
        }
        peer.join().unwrap();
    }
}

#[test]
fn rescanning_preserves_addresses_in_use_and_recovers_after_an_error() {
    let mut home = HomeDevices::new(Ok(vec![DiscoveryResult {
        instance: "848._avdecc._tcp.local".into(),
        host: "848.local".into(),
        port: 17221,
        addresses: vec!["192.168.1.50".into()],
        txt: Vec::new(),
    }]));
    home.hosts.insert("192.168.1.51".into());
    home.update(Err("temporarily unavailable".into()));
    assert_eq!(home.devices.len(), 1);
    assert!(home.discovery_error.is_some());
    home.update(Ok(Vec::new()));
    assert!(home.devices.is_empty());
    assert!(home.discovery_error.is_none());
    assert!(home.hosts.contains("192.168.1.50"));
    assert!(home.hosts.contains("192.168.1.51"));
}

#[test]
fn console_write_route_enforces_authorization_and_batch_parsing_before_network_io() {
    let scope = ServerScope::Configured("192.0.2.1".into());
    let hub = MeterHub::default();
    for (origin, body, status) in [
        (
            Some("https://example.test"),
            "token=secret&changes=mute:input:0:00:1",
            403,
        ),
        (None, "token=secret&changes=mute:input:0:00:1", 403),
        (Some("http://127.0.0.1:8480"), "token=wrong", 403),
        (
            Some("http://127.0.0.1:8480"),
            "token=secret&changes=bad",
            400,
        ),
        (
            Some("http://127.0.0.1:8480"),
            "token=secret&host=192.0.2.2&changes=mute:input:0:00:1",
            400,
        ),
    ] {
        let response = route_browser_request(
            "POST",
            "/api/console/changes",
            body,
            origin,
            &scope,
            "http://127.0.0.1:8480",
            "secret",
            &hub,
            Duration::from_millis(1),
        );
        assert_eq!(response.status, status);
    }
}

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
    let scope = ServerScope::Discovered(Mutex::new(HomeDevices::new(Ok(vec![DiscoveryResult {
        instance: "848._avdecc._tcp.local".to_string(),
        host: "848.local".to_string(),
        port: 17221,
        addresses: vec!["192.168.4.166".to_string(), "fe80::1".to_string()],
        txt: Vec::new(),
    }]))));
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
        monitor: None,
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
        monitor: None,
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

#[test]
fn output_trim_routes_use_the_existing_worker_and_wait_for_verified_results() {
    // No device HTTP listener: an active worker must supply its cached identity.
    let host = "127.0.0.1:1".to_string();
    let hub = MeterHub::default();
    let (stop_sender, stop_receiver) = mpsc::channel();
    let (state_sender, _state_receiver) = mpsc::channel();
    let (write_sender, write_receiver) = mpsc::channel();
    let feed = Arc::new(MixerMeterFeed::default());
    hub.workers.lock().unwrap().insert(
        host.clone(),
        MeterWorker {
            target_entity_id: 0x0001_f2ff_fefe_b9e2,
            stop_sender,
            state_sender,
            write_sender,
            pending_stop: None,
            meters: Arc::clone(&feed),
        },
    );
    let worker = thread::spawn(move || {
        for headphones in [false, true] {
            let SessionWriteRequest::OutputTrim(request) =
                write_receiver.recv_timeout(Duration::from_secs(2)).unwrap()
            else {
                panic!("expected an output trim")
            };
            assert!(request.deadline > Instant::now());
            assert_eq!(request.trim, OutputTrim::Decibels(-18));
            assert!(matches!(
                (headphones, request.output),
                (false, OutputTrimTarget::Line(1)) | (true, OutputTrimTarget::Headphones(1))
            ));
            request
                .reply
                .send(if headphones {
                    Err("device readback differs".into())
                } else {
                    Ok(())
                })
                .unwrap();
        }
    });
    let scope = ServerScope::Configured(host.clone());
    for (path, expected) in [
        ("/api/outputs/line-trim", 200),
        ("/api/outputs/headphone-trim", 502),
    ] {
        let response = route_browser_request(
            "POST",
            path,
            "token=secret&output=1&trim_db=-18",
            Some("http://127.0.0.1:8480"),
            &scope,
            "http://127.0.0.1:8480",
            "secret",
            &hub,
            Duration::from_secs(2),
        );
        assert_eq!(response.status, expected, "{}", response.body);
        assert!(response.body.contains(if expected == 200 {
            "trim verified"
        } else {
            "readback differs"
        }));
        assert!(
            stop_receiver.try_recv().is_err(),
            "output routes must not stop the meter session"
        );
        assert!(Arc::ptr_eq(
            &hub.existing_feed(&host).unwrap().unwrap(),
            &feed
        ));
    }
    worker.join().unwrap();
}

#[test]
fn timed_out_stop_keeps_the_worker_registered_until_it_actually_closes() {
    let hub = MeterHub::default();
    let (stop_sender, stop_receiver) = mpsc::channel();
    let (state_sender, _state_receiver) = mpsc::channel();
    let (write_sender, _write_receiver) = mpsc::channel();
    let meters = Arc::new(MixerMeterFeed::default());
    hub.workers.lock().unwrap().insert(
        "device".into(),
        MeterWorker {
            target_entity_id: 0x0001_f2ff_fefe_b9e2,
            stop_sender,
            state_sender,
            write_sender,
            pending_stop: None,
            meters: Arc::clone(&meters),
        },
    );
    assert!(hub.stop("device", Duration::ZERO).is_err());
    assert!(hub.workers.lock().unwrap().contains_key("device"));
    assert!(
        hub.start("device", 1, Duration::ZERO).is_err(),
        "do not start a second session while the first is closing"
    );
    stop_receiver.recv().unwrap().send(()).unwrap();
    assert!(hub.existing_feed("device").unwrap().is_none());
}

#[test]
fn input_and_console_routes_reuse_the_worker_without_device_identity_requests() {
    let host = "127.0.0.1:1".to_string();
    let hub = MeterHub::default();
    let (stop_sender, stop_receiver) = mpsc::channel();
    let (state_sender, _state_receiver) = mpsc::channel();
    let (write_sender, write_receiver) = mpsc::channel();
    hub.workers.lock().unwrap().insert(
        host.clone(),
        MeterWorker {
            target_entity_id: 0x0001_f2ff_fefe_b9e2,
            stop_sender,
            state_sender,
            write_sender,
            pending_stop: None,
            meters: Arc::new(MixerMeterFeed::default()),
        },
    );
    let worker = thread::spawn(move || {
        for step in 0..4 {
            match write_receiver.recv_timeout(Duration::from_secs(2)).unwrap() {
                SessionWriteRequest::InputGain(request) => {
                    assert!(request.deadline > Instant::now());
                    assert_eq!(request.gain_db, 12);
                    assert!(matches!(
                        (step, request.input),
                        (0, InputGainTarget::Preamp(2)) | (1, InputGainTarget::Line(6))
                    ));
                    request
                        .reply
                        .send(if step == 0 {
                            Ok(())
                        } else {
                            Err("readback differs".into())
                        })
                        .unwrap();
                }
                SessionWriteRequest::Console(request) => {
                    assert!(step >= 2);
                    assert!(request.deadline > Instant::now());
                    assert_eq!(request.changes.len(), 1);
                    request
                        .reply
                        .send(if step == 2 {
                            Ok((1, MonitorState::default()))
                        } else {
                            Err(ConsoleWriteError {
                                applied: 0,
                                conflict: true,
                                message: "conflict: device changed".into(),
                            })
                        })
                        .unwrap();
                }
                _ => panic!("unexpected request"),
            }
        }
    });
    let scope = ServerScope::Configured(host);
    for (path, body, status) in [
        (
            "/api/inputs/gain",
            "token=secret&bank=mic&input=2&gain_db=12",
            200,
        ),
        (
            "/api/inputs/gain",
            "token=secret&bank=line&input=6&gain_db=12",
            502,
        ),
        (
            "/api/console/changes",
            "token=secret&changes=level:main:10:00404de6:-6",
            200,
        ),
        (
            "/api/console/changes",
            "token=secret&changes=master:aux-0:0:01000000:-6",
            409,
        ),
    ] {
        let response = route_browser_request(
            "POST",
            path,
            body,
            Some("http://127.0.0.1:8480"),
            &scope,
            "http://127.0.0.1:8480",
            "secret",
            &hub,
            Duration::from_secs(2),
        );
        assert_eq!(response.status, status, "{}", response.body);
        assert!(
            stop_receiver.try_recv().is_err(),
            "no slider may stop its session"
        );
    }
    worker.join().unwrap();
}

#[test]
fn input_gain_route_checks_authorization_host_and_ranges_before_network_io() {
    let scope = ServerScope::Configured("127.0.0.1:1".into());
    let hub = MeterHub::default();
    for (origin, body, status) in [
        (None, "token=secret&bank=mic&input=0&gain_db=12", 403),
        (
            Some("https://example.test"),
            "token=secret&bank=mic&input=0&gain_db=12",
            403,
        ),
        (
            Some("http://127.0.0.1:8480"),
            "token=wrong&bank=mic&input=0&gain_db=12",
            403,
        ),
        (
            Some("http://127.0.0.1:8480"),
            "token=secret&bank=mic&input=0&gain_db=75",
            400,
        ),
        (
            Some("http://127.0.0.1:8480"),
            "token=secret&bank=line&input=0&gain_db=21",
            400,
        ),
        (
            Some("http://127.0.0.1:8480"),
            "token=secret&bank=mic&input=0&gain_db=-1",
            400,
        ),
        (
            Some("http://127.0.0.1:8480"),
            "token=secret&bank=line&input=0&gain_db=1.5",
            400,
        ),
        (
            Some("http://127.0.0.1:8480"),
            "token=secret&bank=output&input=0&gain_db=12",
            400,
        ),
        (
            Some("http://127.0.0.1:8480"),
            "token=secret&bank=mic&input=65536&gain_db=12",
            400,
        ),
        (
            Some("http://127.0.0.1:8480"),
            "token=secret&bank=mic&input=0&gain_db=12&host=other",
            400,
        ),
    ] {
        let response = route_browser_request(
            "POST",
            "/api/inputs/gain",
            body,
            origin,
            &scope,
            "http://127.0.0.1:8480",
            "secret",
            &hub,
            Duration::from_millis(1),
        );
        assert_eq!(response.status, status, "{}", response.body);
    }
}
