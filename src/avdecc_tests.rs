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
