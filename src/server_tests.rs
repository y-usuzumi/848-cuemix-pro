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
fn limits_proxying_to_the_configured_device() {
    let params = parse_query("host=192.168.4.166");
    assert_eq!(allowed_host(&params, "192.168.4.166"), Ok("192.168.4.166"));
    let params = parse_query("host=192.168.4.1");
    assert!(allowed_host(&params, "192.168.4.166").is_err());
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
