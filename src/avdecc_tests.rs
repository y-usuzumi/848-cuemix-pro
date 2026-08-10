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
