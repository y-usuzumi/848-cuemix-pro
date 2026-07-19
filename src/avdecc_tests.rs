use super::*;

#[test]
fn decodes_complete_version_zero_frames() {
    let bytes = [
        0,
        APP_ENTITY_GUID_RESPONSE,
        0,
        2,
        1,
        2,
        3,
        4,
        5,
        6,
        0xaa,
        0xbb,
    ];
    assert_eq!(
        decode_complete_v0_frames(&bytes),
        vec![AppFrame {
            version: 0,
            message_type: APP_ENTITY_GUID_RESPONSE,
            address: [1, 2, 3, 4, 5, 6],
            payload: vec![0xaa, 0xbb],
        }]
    );
}

#[test]
fn does_not_interpret_partial_or_newer_frames_as_version_zero() {
    assert!(decode_complete_v0_frames(&[0, 2, 0]).is_empty());
    assert!(decode_complete_v0_frames(&[1, 0, 0, 0]).is_empty());
}

#[test]
fn parses_proxy_addresses() {
    let address = parse_proxy_address("[fe80::1]:17221").unwrap();
    assert_eq!(address.host_header, "[fe80::1]:17221");
    assert_eq!(address.socket_address, "[fe80::1]:17221");
}

#[test]
fn rejects_unsafe_proxy_paths() {
    assert!(validate_proxy_path("/\r\nInjected: yes").is_err());
    assert!(validate_proxy_path("//other-host").is_err());
}
