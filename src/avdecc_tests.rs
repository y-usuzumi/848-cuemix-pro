use super::*;

#[test]
fn decodes_complete_version_zero_frames() {
    let bytes = [
        0,
        APP_ENTITY_ID_RESPONSE,
        0,
        2,
        1,
        2,
        3,
        4,
        5,
        6,
        0,
        0,
        0xaa,
        0xbb,
    ];
    assert_eq!(
        decode_complete_v0_frames(&bytes),
        vec![AppFrame {
            version: 0,
            message_type: APP_ENTITY_ID_RESPONSE,
            address: [1, 2, 3, 4, 5, 6],
            reserved: 0,
            payload: vec![0xaa, 0xbb],
        }]
    );
}

#[test]
fn encodes_the_complete_version_zero_app_header() {
    let frame = AppFrame {
        version: 0,
        message_type: APP_ENTITY_ID_REQUEST,
        address: [1, 2, 3, 4, 5, 6],
        reserved: 0,
        payload: vec![0; 8],
    };
    assert_eq!(
        frame.encode().unwrap(),
        vec![0, 1, 0, 8, 1, 2, 3, 4, 5, 6, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]
    );
}

#[test]
fn does_not_interpret_partial_or_newer_frames_as_version_zero() {
    assert!(decode_complete_v0_frames(&[0, 2, 0]).is_empty());
    assert!(decode_complete_v0_frames(&[1, 0, 0, 0]).is_empty());
    assert!(decode_complete_v0_frames(&[0, 7, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0]).is_empty());
}

#[test]
fn parses_proxy_addresses() {
    let address = parse_proxy_address("[fe80::1]:17221").unwrap();
    assert_eq!(address.host_header, "[fe80::1]:17221");
    assert_eq!(address.socket_address, "[fe80::1]:17221");

    let scoped = parse_proxy_address("[fe80::1%eth2]:17221").unwrap();
    assert_eq!(scoped.host_header, "[fe80::1%25eth2]:17221");
    assert_eq!(scoped.socket_address, "[fe80::1%eth2]:17221");
}

#[test]
fn parses_mac_addresses() {
    assert_eq!(
        avdecc_transport::parse_mac_address("aa:bb:cc:dd:ee:ff"),
        Ok([0xaa, 0xbb, 0xcc, 0xdd, 0xee, 0xff])
    );
}

#[test]
fn requires_a_version_zero_entity_id_response() {
    let frame = AppFrame {
        version: 1,
        message_type: APP_ENTITY_ID_RESPONSE,
        address: [1, 2, 3, 4, 5, 6],
        reserved: 0,
        payload: vec![0; 8],
    };
    assert!(!is_entity_id_response(&frame, [1, 2, 3, 4, 5, 6]));
}

#[test]
fn preserves_previewed_bytes_for_an_entity_id_request() {
    let frame = AppFrame {
        version: 0,
        message_type: APP_LINK_UP,
        address: [6, 5, 4, 3, 2, 1],
        reserved: 0,
        payload: Vec::new(),
    };
    let bytes = frame.encode().unwrap();
    let mut preview = bytes[..7].to_vec();
    let mut buffered = bytes[..7].to_vec();
    append_preview_bytes(&mut preview, &mut buffered, &bytes[7..], true);
    assert_eq!(preview, bytes);
    assert_eq!(buffered, bytes);
}

#[test]
fn rejects_unsafe_proxy_paths() {
    assert!(validate_proxy_path("/\r\nInjected: yes").is_err());
    assert!(validate_proxy_path("//other-host").is_err());
}

#[test]
fn restricts_mixer_faders_to_capture_validated_routes_and_levels() {
    assert_eq!(
        MixerFader::parse("main-1-2", "host-11-12")
            .unwrap()
            .property_and_index(),
        (0x841a, 0x0a00)
    );
    assert_eq!(
        MixerFader::parse("headphone-mix", "host-11-12")
            .unwrap()
            .property_and_index(),
        (0x83f8, 0x0a00)
    );
    assert_eq!(
        MixerFader::parse("main-1-2", "line-in-5-6")
            .unwrap()
            .property_and_index(),
        (0x841a, 0x1000)
    );
    assert!(MixerFader::parse("main-1-2", "mic-1-2").is_err());
    assert_eq!(
        MixerLevel::parse("-12").unwrap().encoded_value(),
        0x0040_4de6
    );
    assert_eq!(
        MixerLevel::parse("-60").unwrap().encoded_value(),
        0x0000_4189
    );
    assert!(MixerLevel::parse("-24").is_err());
}

#[test]
fn discovers_line_and_headphone_outputs_from_their_distinct_vendor_properties() {
    let state = parse_vendor_state_records(&[
        0x13, 0x88, 0x00, 0x00, 0x01, 0x23, // Line Out 1: -35 dB
        0x13, 0x88, 0x00, 0x03, 0x01, 0x2a, // Line Out 4: -42 dB
        0x93, 0xac, 0x00, 0x00, 0x04, 0x13, 0xad, 0x01, 0x00, // Line Out 1 meter
        0x93, 0xac, 0x00, 0x03, 0x04, 0x13, 0xad, 0x00, 0x03, // Line Out 4 meter
        0x13, 0x9d, 0x00, 0x00, 0x01, 0x00, // unrelated four-channel state
        0x13, 0xb4, 0x00, 0x00, 0x04, 0x13, 0xad, 0x02, 0x00, // Phones 1 meter
        0x13, 0xb4, 0x00, 0x01, 0x04, 0x13, 0xad, 0x03, 0x02, // Phones 2 meter
        0x13, 0xb7, 0x00, 0x00, 0x01, 0x64, // Phones 1 L: -infinity
        0x13, 0xb7, 0x00, 0x01, 0x01, 0x64, // Phones 1 R: -infinity
        0x13, 0xb7, 0x00, 0x02, 0x01, 0x32, // Phones 2 L: -50 dB
        0x13, 0xb7, 0x00, 0x03, 0x01, 0x32, // Phones 2 R: -50 dB
    ])
    .unwrap();
    assert_eq!(
        line_outputs_from_state(&state).unwrap(),
        vec![
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
                meter_path: Some(MeterPath {
                    property_id: 0x13ad,
                    record_index: 0,
                    channel_index: 3,
                }),
            },
        ]
    );
    assert_eq!(
        headphone_outputs_from_state(&state).unwrap(),
        vec![
            HeadphoneOutput {
                channel_indices: [0, 1],
                attenuation: [100, 100],
                meter_path: Some(MeterPath {
                    property_id: 0x13ad,
                    record_index: 2,
                    channel_index: 0,
                }),
            },
            HeadphoneOutput {
                channel_indices: [2, 3],
                attenuation: [50, 50],
                meter_path: Some(MeterPath {
                    property_id: 0x13ad,
                    record_index: 3,
                    channel_index: 2,
                }),
            },
        ]
    );
}

#[test]
fn discovers_sorted_line_input_gains_and_polarities() {
    let state = parse_vendor_state_records(&[
        0x13, 0xb2, 0x00, 0x01, 0x01, 0x14, // Line In 6: +20 dB
        0x13, 0xb3, 0x00, 0x00, 0x01, 0x01, // Line In 5: inverted
        0x13, 0xb2, 0x00, 0x00, 0x01, 0x07, // Line In 5: +7 dB
        0x13, 0xb3, 0x00, 0x01, 0x01, 0x00, // Line In 6: normal
    ])
    .unwrap();
    assert_eq!(
        line_inputs_from_state(&state).unwrap(),
        vec![
            LineInput {
                channel_index: 0,
                gain_db: 7,
                phase_inverted: true,
            },
            LineInput {
                channel_index: 1,
                gain_db: 20,
                phase_inverted: false,
            },
        ]
    );
    assert_eq!(
        one_byte_property_payload(LINE_INPUT_PHASE_PROPERTY, 7, 1),
        [0x13, 0xb3, 0x00, 0x07, 0x01, 0x01]
    );
}

#[test]
fn rejects_mismatched_or_unsafe_line_input_state() {
    let mismatched = parse_vendor_state_records(&[
        0x13, 0xb2, 0x00, 0x00, 0x01, 0x00, 0x13, 0xb3, 0x00, 0x01, 0x01, 0x00,
    ])
    .unwrap();
    assert!(line_inputs_from_state(&mismatched).is_err());

    let invalid_gain = parse_vendor_state_records(&[
        0x13, 0xb2, 0x00, 0x00, 0x01, 0x15, 0x13, 0xb3, 0x00, 0x00, 0x01, 0x00,
    ])
    .unwrap();
    assert!(line_inputs_from_state(&invalid_gain).is_err());

    let invalid_phase = parse_vendor_state_records(&[
        0x13, 0xb2, 0x00, 0x00, 0x01, 0x00, 0x13, 0xb3, 0x00, 0x00, 0x01, 0x02,
    ])
    .unwrap();
    assert!(line_inputs_from_state(&invalid_phase).is_err());
}

#[test]
fn discovers_single_phone_interfaces_and_rejects_wrapped_channel_pairs() {
    let single = vec![
        VendorStateRecord {
            property_id: HEADPHONE_TRIM_PROPERTY,
            property_index: 8,
            value: vec![6],
        },
        VendorStateRecord {
            property_id: HEADPHONE_TRIM_PROPERTY,
            property_index: 9,
            value: vec![6],
        },
    ];
    assert_eq!(
        headphone_outputs_from_state(&single).unwrap(),
        vec![HeadphoneOutput {
            channel_indices: [8, 9],
            attenuation: [6, 6],
            meter_path: None,
        }]
    );

    let wrapped = vec![
        VendorStateRecord {
            property_id: HEADPHONE_TRIM_PROPERTY,
            property_index: 0,
            value: vec![6],
        },
        VendorStateRecord {
            property_id: HEADPHONE_TRIM_PROPERTY,
            property_index: u16::MAX,
            value: vec![6],
        },
    ];
    assert!(headphone_outputs_from_state(&wrapped).is_err());
}

#[test]
fn encodes_one_linked_stereo_headphone_trim_and_rejects_unsafe_state() {
    let output = HeadphoneOutput {
        channel_indices: [2, 3],
        attenuation: [0, 0],
        meter_path: None,
    };
    assert_eq!(
        output_trim_payload(
            HEADPHONE_TRIM_PROPERTY,
            &output.channel_indices,
            OutputTrim::Decibels(-50)
        ),
        vec![0x13, 0xb7, 0x00, 0x02, 0x01, 0x32, 0x13, 0xb7, 0x00, 0x03, 0x01, 0x32,]
    );
    assert_eq!(
        output_trim_payload(
            HEADPHONE_TRIM_PROPERTY,
            &output.channel_indices,
            OutputTrim::NegativeInfinity
        ),
        vec![0x13, 0xb7, 0x00, 0x02, 0x01, 0x64, 0x13, 0xb7, 0x00, 0x03, 0x01, 0x64,]
    );
    assert_eq!(
        output_trim_payload(LINE_OUTPUT_TRIM_PROPERTY, &[3], OutputTrim::Decibels(-42)),
        vec![0x13, 0x88, 0x00, 0x03, 0x01, 0x2a]
    );
    assert_eq!(
        OutputTrim::parse("-inf").unwrap(),
        OutputTrim::NegativeInfinity
    );
    assert_eq!(OutputTrim::parse("-50").unwrap(), OutputTrim::Decibels(-50));
    assert!(OutputTrim::parse("-100").is_err());

    let incomplete = vec![VendorStateRecord {
        property_id: HEADPHONE_TRIM_PROPERTY,
        property_index: 0,
        value: vec![12],
    }];
    assert!(headphone_outputs_from_state(&incomplete).is_err());

    let nonconsecutive = vec![
        VendorStateRecord {
            property_id: HEADPHONE_TRIM_PROPERTY,
            property_index: 0,
            value: vec![12],
        },
        VendorStateRecord {
            property_id: HEADPHONE_TRIM_PROPERTY,
            property_index: 2,
            value: vec![12],
        },
    ];
    assert!(headphone_outputs_from_state(&nonconsecutive).is_err());

    let invalid_attenuation = vec![
        VendorStateRecord {
            property_id: HEADPHONE_TRIM_PROPERTY,
            property_index: 0,
            value: vec![101],
        },
        VendorStateRecord {
            property_id: HEADPHONE_TRIM_PROPERTY,
            property_index: 1,
            value: vec![101],
        },
    ];
    assert!(headphone_outputs_from_state(&invalid_attenuation).is_err());
}

#[test]
fn derives_the_proxy_ethernet_address_from_an_entity_id() {
    assert_eq!(
        ethernet_address_from_entity_id(0x0001_f2ff_fefe_b9e2),
        [0x00, 0x01, 0xf2, 0xfe, 0xb9, 0xe2]
    );
}

#[test]
fn parses_capture_shaped_meter_pages_and_maps_fader_slots() {
    let records = parse_mixer_meter_page(&[
        0x3c, 0x21, // opaque captured page counter
        0x13, 0xad, 0x00, 0x04, 0xff, 0xff, 0x7e, 0x80, 0x13, 0xb9, 0x02, 0x02, 0xb4, 0xb4,
    ])
    .unwrap();
    assert_eq!(records.len(), 2);
    assert_eq!(records[0].property_id, 0x13ad);
    assert_eq!(records[0].index, 0);
    assert_eq!(records[0].values, vec![0xffff, 0x7e80]);
    assert_eq!(records[0].channels(), vec![0xff, 0xff, 0x7e, 0x80]);
    assert_eq!(records[1].property_id, 0x13b9);
    assert_eq!(records[1].index, 2);
    assert_eq!(records[1].channels(), vec![0xb4, 0xb4]);
    assert_eq!(MixerFader::MainHost11To12.meter_slot(), (0x13ad, 0, 5));
    assert_eq!(MixerFader::MainLineIn5To6.meter_slot(), (0x13ad, 0, 10));
}

#[test]
fn rejects_truncated_or_odd_meter_records() {
    assert!(parse_mixer_meter_page(&[0x3c]).is_err());
    assert!(parse_mixer_meter_page(&[0x3c, 0x21, 0x13, 0xad, 0, 1, 0]).is_err());
}

#[test]
fn meter_feed_notifies_each_snapshot_and_closure() {
    let feed = Arc::new(MixerMeterFeed::default());
    update_mixer_meters(
        &feed,
        vec![MixerMeterRecord {
            property_id: 0x138c,
            index: 0,
            values: vec![0x6a6f],
        }],
        None,
    );
    let update = feed
        .wait_after(0, Duration::from_millis(0))
        .unwrap()
        .expect("published meter snapshot");
    assert_eq!(update.revision, 1);
    assert_eq!(update.meters.records[0].channels(), vec![106, 111]);
    assert!(!update.closed);

    feed.close();
    let closed = feed
        .wait_after(update.revision, Duration::from_millis(0))
        .unwrap()
        .expect("feed closure");
    assert!(closed.closed);
}

// A local protocol peer exercises the actual TCP session and worker channels.
// State pages are delayed enough to require an intervening meter request.
fn simulated_meter_peer_with_monitor(reject_write: bool) -> (String, thread::JoinHandle<Vec<u8>>) {
    simulated_meter_peer_with_lost_updates(reject_write, false)
}

fn simulated_meter_peer_with_lost_updates(
    reject_write: bool,
    lose_updates: bool,
) -> (String, thread::JoinHandle<Vec<u8>>) {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let address = listener.local_addr().unwrap().to_string();
    let peer = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        stream
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        stream.set_nodelay(true).unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            stream.read_exact(&mut byte).unwrap();
            header.push(byte[0]);
        }
        stream.write_all(b"HTTP/1.1 200 OK\r\n\r\n").unwrap();
        let mut trace = Vec::new();
        let mut prior_sequence = 0u16;
        let mut state_sequence = 0u16;
        let mut generation = 0u8;
        let mut terminal = false;
        let mut attenuation = 30u8;
        let mut muted = 0u8;
        let mut mono = 0u8;
        let mut talking = 0u8;
        let mut external_sent = false;
        loop {
            let mut header = [0; 12];
            if let Err(error) = stream.read_exact(&mut header) {
                assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
                return trace;
            }
            let mut payload = vec![0; u16::from_be_bytes([header[2], header[3]]) as usize];
            stream.read_exact(&mut payload).unwrap();
            let mut frame = decode_app_frame(&header, payload).unwrap();
            if frame.message_type == APP_ENTITY_ID_REQUEST {
                frame.message_type = APP_ENTITY_ID_RESPONSE;
                frame.payload = 42u64.to_be_bytes().to_vec();
                stream.write_all(&frame.encode().unwrap()).unwrap();
                continue;
            }
            assert_eq!(frame.message_type, APP_AVDECC_FROM_APC);
            let sequence = u16::from_be_bytes([frame.payload[20], frame.payload[21]]);
            assert_eq!(sequence, prior_sequence.wrapping_add(1));
            prior_sequence = sequence;
            let protocol = frame.payload[27];
            assert!(matches!(protocol, 1 | 3 | 4));
            let data = if protocol == 1 {
                if frame.payload.len() == 28 {
                    generation += 1;
                    state_sequence = sequence;
                    trace.push(1);
                    thread::sleep(Duration::from_millis(25));
                    terminal = true;
                    vec![
                        0x03,
                        0xfb,
                        0,
                        0,
                        1,
                        generation,
                        0x13,
                        0x93,
                        0,
                        0,
                        1,
                        attenuation,
                        0x13,
                        0x9a,
                        0,
                        0,
                        1,
                        mono,
                        0x13,
                        0x9b,
                        0,
                        0,
                        1,
                        muted,
                        0x13,
                        0xa3,
                        0,
                        0,
                        1,
                        talking,
                        0x13,
                        0x94,
                        0,
                        0,
                        2,
                        0,
                        3,
                        0x13,
                        0xb6,
                        0,
                        0,
                        1,
                        0,
                        0x93,
                        0xb9,
                        0,
                        0,
                        4,
                        0,
                        0,
                        0,
                        0,
                        0x93,
                        0xb9,
                        0,
                        1,
                        4,
                        0,
                        0,
                        0,
                        0,
                        0x13,
                        0x88,
                        0,
                        0,
                        1,
                        0,
                        0x13,
                        0x88,
                        0,
                        1,
                        1,
                        0,
                    ]
                } else {
                    assert_eq!(
                        &frame.payload[28..],
                        &state_sequence.to_be_bytes(),
                        "ACK must refer to the state page, not the intervening meter sequence"
                    );
                    trace.push(2);
                    state_sequence = sequence;
                    if terminal || lose_updates {
                        terminal = false;
                        Vec::new()
                    } else {
                        terminal = true;
                        generation = generation.wrapping_add(1);
                        vec![
                            0x03,
                            0xfb,
                            0,
                            0,
                            1,
                            generation,
                            0x13,
                            0x93,
                            0,
                            0,
                            1,
                            attenuation,
                        ]
                    }
                }
            } else if protocol == 3 {
                trace.push(3);
                assert_eq!(&frame.payload[30..33], &[0, 0, 1]);
                if !reject_write {
                    match &frame.payload[28..30] {
                        [0x13, 0x93] => attenuation = frame.payload[33],
                        [0x13, 0x9b] => muted = frame.payload[33],
                        [0x13, 0x9a] => mono = frame.payload[33],
                        [0x13, 0xa3] => talking = frame.payload[33],
                        _ => panic!("unexpected monitor setter"),
                    }
                }
                Vec::new()
            } else {
                trace.push(4);
                vec![0, 0, 0x13, 0xad, 0, 2, 0x6a, 0x6f]
            };
            frame.message_type = APP_AVDECC_FROM_APS;
            frame.payload.truncate(28);
            frame.payload[1] = 7;
            frame.payload[2..4].copy_from_slice(&(16u16 + data.len() as u16).to_be_bytes());
            frame.payload.extend(data);
            if protocol == 3 && reject_write {
                frame.payload[2] |= 8;
            }
            stream.write_all(&frame.encode().unwrap()).unwrap();
            if protocol == 4 {
                if !external_sent {
                    // A front-panel update arrives between the two meter pages.
                    // Wrong sequence/controller/length must not become state.
                    let mut event = frame.clone();
                    event.payload.truncate(28);
                    event.payload[27] = 1;
                    event.payload[20..22].copy_from_slice(&state_sequence.to_be_bytes());
                    event.payload[2..4].copy_from_slice(&22u16.to_be_bytes());
                    event.payload.extend([0x13, 0x93, 0, 0, 1, 40]);
                    let mut wrong = event.clone();
                    wrong.payload[20..22].copy_from_slice(&65500u16.to_be_bytes());
                    wrong.payload[33] = 99;
                    stream.write_all(&wrong.encode().unwrap()).unwrap();
                    attenuation = if lose_updates { 45 } else { 40 };
                    if !lose_updates {
                        stream.write_all(&event.encode().unwrap()).unwrap();
                    }
                    external_sent = true;
                }
                stream.write_all(&frame.encode().unwrap()).unwrap();
            }
        }
    });
    (address, peer)
}

#[test]
fn repeated_fresh_reads_share_the_meter_session_and_preserve_state_ack_sequences() {
    let (address, peer) = simulated_meter_peer_with_monitor(false);
    let timeout = Duration::from_secs(2);
    let worker = start_mixer_meter_worker(address, 0x0001_f2ff_fefe_b9e2, timeout);
    let mut revision = worker
        .meters
        .wait_after(0, timeout)
        .unwrap()
        .unwrap()
        .revision;
    let mut previous_generation = 0;
    for _ in 0..2 {
        let (reply, receiver) = mpsc::channel();
        worker
            .state_sender
            .send(VendorSnapshotRequest {
                deadline: Instant::now() + timeout,
                reply,
            })
            .unwrap();
        let state = receiver.recv_timeout(timeout).unwrap().unwrap();
        let generation = state
            .0
            .iter()
            .find(|r| r.property_id == 0x03fb)
            .unwrap()
            .value[0];
        assert!(
            generation > previous_generation,
            "each request must read fresh incremental state"
        );
        previous_generation = generation;
        let update = worker
            .meters
            .wait_after(revision, timeout)
            .unwrap()
            .unwrap();
        assert!(!update.closed);
        assert!(
            update.revision > revision,
            "meters must progress during a state read"
        );
        revision = update.revision;
    }
    // Requests that expire in the queue must not initiate another inventory.
    let (reply, receiver) = mpsc::channel();
    worker
        .state_sender
        .send(VendorSnapshotRequest {
            deadline: Instant::now() - timeout,
            reply,
        })
        .unwrap();
    assert!(receiver.recv_timeout(timeout).unwrap().is_err());
    assert!(!worker.meters.snapshot().unwrap().closed);
    let (reply, stopped) = mpsc::channel();
    worker.stop_sender.send(reply).unwrap();
    stopped.recv_timeout(timeout).unwrap();
    assert!(worker.meters.snapshot().unwrap().closed);
    let trace = peer.join().unwrap();
    assert_eq!(trace.iter().filter(|&&kind| kind == 1).count(), 3);
    assert!(
        !trace.contains(&3),
        "background updates must never send setters"
    );
    assert!(trace.iter().filter(|&&kind| kind == 4).count() >= 2);
}

#[test]
fn front_panel_writes_read_back_device_latches_without_touching_level_or_selection() {
    let (address, peer) = simulated_meter_peer_with_monitor(false);
    let timeout = Duration::from_secs(2);
    let meters = Arc::new(MixerMeterFeed::default());
    let mut session = MixerMeterSession::open(&address, 0x0001_f2ff_fefe_b9e2, timeout).unwrap();
    session.poll(timeout).unwrap();
    let before = session.monitor_revision;
    let changes = parse_changes(
        "monitor-mute:monitor:0:00:1;monitor-mono:monitor:0:00:1;monitor-talk:monitor:0:00:1",
    )
    .unwrap();
    let (count, state) = session
        .write_monitor(&changes, Instant::now() + timeout, &meters)
        .unwrap();
    assert_eq!(count, 3);
    assert!(state.revision > before);
    for property in [0x139b, 0x139a, 0x13a3] {
        assert_eq!(
            state
                .records
                .iter()
                .find(|r| r.property_id == property)
                .unwrap()
                .value,
            [1]
        );
    }
    assert_eq!(session.state.get(&(0x1393, 0)), Some(&vec![40]));
    assert_eq!(session.state.get(&(0x13b6, 0)), Some(&vec![0]));
    let stale = parse_changes("monitor-mute:monitor:0:01:0;monitor-mono:monitor:0:00:1").unwrap();
    assert!(
        session
            .write_monitor(&stale, Instant::now() + timeout, &meters)
            .unwrap_err()
            .conflict
    );
    let no_op = parse_changes(
        "monitor-mute:monitor:0:01:1;monitor-mono:monitor:0:01:1;monitor-talk:monitor:0:01:1",
    )
    .unwrap();
    assert_eq!(
        session
            .write_monitor(&no_op, Instant::now() + timeout, &meters)
            .unwrap()
            .0,
        0
    );
    // Front-panel changes must update streamed state and its revision.
    for property in [0x139a_u16, 0x13a3] {
        let revision = session.monitor_revision;
        let [hi, lo] = property.to_be_bytes();
        session.apply_state(&[hi, lo, 0, 0, 1, 0]).unwrap();
        assert!(session.monitor_revision > revision);
        assert_eq!(
            session
                .monitor_state()
                .records
                .iter()
                .find(|r| r.property_id == property)
                .unwrap()
                .value,
            [0]
        );
    }
    drop(session);
    assert_eq!(
        peer.join()
            .unwrap()
            .iter()
            .filter(|&&kind| kind == 3)
            .count(),
        3
    );
}

#[test]
fn monitor_events_and_writes_share_one_session_with_readback_and_no_retries() {
    for reject in [false, true] {
        let (address, peer) = simulated_meter_peer_with_monitor(reject);
        let timeout = Duration::from_secs(1);
        let meters = Arc::new(MixerMeterFeed::default());
        let mut session =
            MixerMeterSession::open(&address, 0x0001_f2ff_fefe_b9e2, timeout).unwrap();
        session.poll(timeout).unwrap();
        assert_eq!(
            session.state.get(&(0x1393, 0)),
            Some(&vec![40]),
            "event between meter pages updates the level"
        );
        let before = session.monitor_revision;
        let changes = parse_changes("monitor-level:monitor:0:28:-31").unwrap();
        let result = session.write_monitor(&changes, Instant::now() + timeout, &meters);
        if reject {
            assert!(result.unwrap_err().message.contains("status 1"));
        } else {
            let (count, state) = result.unwrap();
            assert_eq!(count, 1);
            assert!(state.json().contains("\"1f\""));
            assert!(state.revision > before);
            session.poll(timeout).unwrap();
            assert!(
                session
                    .write_monitor(&changes, Instant::now() + timeout, &meters)
                    .unwrap_err()
                    .conflict
            );
            let no_op = parse_changes("monitor-level:monitor:0:1f:-31").unwrap();
            assert_eq!(
                session
                    .write_monitor(&no_op, Instant::now() + timeout, &meters)
                    .unwrap()
                    .0,
                0
            );
            assert!(session
                .write_monitor(&no_op, Instant::now() - timeout, &meters)
                .is_err());
        }
        drop(session);
        let trace = peer.join().unwrap();
        assert_eq!(
            trace.iter().filter(|&&p| p == 1).count(),
            if reject { 2 } else { 6 }
        );
        assert_eq!(
            trace.iter().filter(|&&p| p == 3).count(),
            1,
            "no-op, conflict, expiry and rejection never retry a setter"
        );
    }
}

#[test]
fn monitor_recovery_reads_device_even_when_incremental_replies_are_empty() {
    let (address, peer) = simulated_meter_peer_with_lost_updates(false, true);
    let timeout = Duration::from_secs(2);
    let worker = start_mixer_meter_worker(address, 0x0001_f2ff_fefe_b9e2, timeout);
    let mut update = worker.meters.wait_after(0, timeout).unwrap().unwrap();
    assert!(update
        .meters
        .monitor
        .as_ref()
        .unwrap()
        .json()
        .contains("\"1e\""));
    let deadline = Instant::now() + timeout;
    while !update
        .meters
        .monitor
        .as_ref()
        .unwrap()
        .json()
        .contains("\"2d\"")
    {
        update = worker
            .meters
            .wait_after(update.revision, refresh_remaining(deadline).unwrap())
            .unwrap()
            .unwrap();
        assert!(update.meters.error.is_none());
    }
    // This recovery happened with no browser GET or write request.
    let (reply, stopped) = mpsc::channel();
    worker.stop_sender.send(reply).unwrap();
    stopped.recv_timeout(timeout).unwrap();
    let trace = peer.join().unwrap();
    assert!(trace.iter().filter(|&&p| p == 1).count() >= 2);
    assert!(!trace.contains(&3));
    assert!(
        trace.windows(3).any(|p| p == [1, 4, 2]),
        "meters continue between inventory pages"
    );
}

#[test]
fn lost_monitor_events_cannot_validate_stale_writes_or_hide_write_readback() {
    let (address, peer) = simulated_meter_peer_with_lost_updates(false, true);
    let timeout = Duration::from_secs(1);
    let meters = Arc::new(MixerMeterFeed::default());
    let mut session = MixerMeterSession::open(&address, 0x0001_f2ff_fefe_b9e2, timeout).unwrap();
    session.poll(timeout).unwrap(); // Hardware changes to -45 without an event.
    session.sync_state(Instant::now() + timeout).unwrap(); // Empty successful reply leaves cached -30.
    assert_eq!(session.state.get(&(0x1393, 0)), Some(&vec![30]));
    let stale = parse_changes("monitor-level:monitor:0:1e:-31").unwrap();
    assert!(
        session
            .write_monitor(&stale, Instant::now() + timeout, &meters)
            .unwrap_err()
            .conflict
    );
    assert_eq!(session.state.get(&(0x1393, 0)), Some(&vec![45]));
    let desired = parse_changes("monitor-level:monitor:0:2d:-44").unwrap();
    let (count, state) = session
        .write_monitor(&desired, Instant::now() + timeout, &meters)
        .unwrap();
    assert_eq!(count, 1);
    assert!(
        state.json().contains("\"2c\""),
        "readback must come from hardware without an incremental event"
    );
    drop(session);
    let trace = peer.join().unwrap();
    assert_eq!(
        trace.iter().filter(|&&p| p == 3).count(),
        1,
        "conflicting write is never sent"
    );
}

#[test]
fn a_monitor_event_after_its_inventory_page_survives_snapshot_commit() {
    let (address, peer) = simulated_meter_peer_with_monitor(false);
    let timeout = Duration::from_secs(1);
    let meters = Arc::new(MixerMeterFeed::default());
    let mut session = MixerMeterSession::open(&address, 0x0001_f2ff_fefe_b9e2, timeout).unwrap();
    // The inventory page contains -30, then the interleaved meter exchange
    // delivers a newer -40 event before the inventory's terminal page.
    session
        .read_state(Instant::now() + timeout, &meters)
        .unwrap();
    assert_eq!(session.state.get(&(0x1393, 0)), Some(&vec![40]));
    assert!(session.snapshot_events.is_none());
    assert!(!meters.snapshot().unwrap().meters.records.is_empty());
    drop(session);
    assert!(!peer.join().unwrap().contains(&3));
}
