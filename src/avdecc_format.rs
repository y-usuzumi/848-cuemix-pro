use super::{
    AppFrame, APP_AVDECC_FROM_APC, APP_AVDECC_FROM_APS, APP_ENTITY_ID_REQUEST,
    APP_ENTITY_ID_RESPONSE, APP_LINK_DOWN, APP_LINK_UP, APP_NOP,
};

pub(super) fn hex_preview(bytes: &[u8], maximum: usize) -> String {
    bytes
        .iter()
        .take(maximum)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(super) fn app_frame_json(frame: &AppFrame) -> String {
    let address = frame
        .address
        .iter()
        .map(|octet| format!("{octet:02x}"))
        .collect::<Vec<_>>()
        .join(":");
    format!(
        "{{\"version\":{},\"message_type\":\"{}\",\"address\":\"{}\",\"reserved\":{},\"payload_bytes\":{},\"payload_preview\":\"{}\"}}",
        frame.version,
        app_message_type_name(frame.message_type),
        address,
        frame.reserved,
        frame.payload.len(),
        hex_preview(&frame.payload, 48)
    )
}

fn app_message_type_name(message_type: u8) -> &'static str {
    match message_type {
        APP_NOP => "nop",
        APP_ENTITY_ID_REQUEST => "entity_id_request",
        APP_ENTITY_ID_RESPONSE => "entity_id_response",
        APP_LINK_UP => "link_up",
        APP_LINK_DOWN => "link_down",
        APP_AVDECC_FROM_APS => "avdecc_from_aps",
        APP_AVDECC_FROM_APC => "avdecc_from_apc",
        0xff => "vendor",
        _ => "unknown",
    }
}
