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

pub(super) fn observed_app_frame_json(frame: &AppFrame, received_ms: u128) -> String {
    let base = app_frame_json(frame);
    let base = base.strip_suffix('}').unwrap_or(&base);
    format!(
        "{base},\"received_ms\":{received_ms},\"avdecc\":{}}}",
        avdecc_payload_json(frame)
    )
}

fn avdecc_payload_json(frame: &AppFrame) -> String {
    if matches!(
        frame.message_type,
        APP_AVDECC_FROM_APS | APP_AVDECC_FROM_APC
    ) && frame.payload.first() == Some(&0xfa)
    {
        return adp_payload_json(frame);
    }
    vendor_state_payload_json(frame).unwrap_or_else(|| "null".to_string())
}

fn adp_payload_json(frame: &AppFrame) -> String {
    let message_type = match frame.payload.get(1).copied() {
        Some(0) => "entity_available",
        Some(1) => "entity_departing",
        Some(2) => "entity_discover",
        _ => "unknown",
    };
    let entity_id = frame
        .payload
        .get(4..12)
        .and_then(|value| value.try_into().ok())
        .map(u64::from_be_bytes)
        .map(|value| format!("\"{value:016x}\""))
        .unwrap_or_else(|| "null".to_string());
    let available_index = frame
        .payload
        .get(36..40)
        .and_then(|value| value.try_into().ok())
        .map(u32::from_be_bytes)
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"protocol\":\"adp\",\"message_type\":\"{message_type}\",\"entity_id\":{entity_id},\"available_index\":{available_index}}}"
    )
}

fn vendor_state_payload_json(frame: &AppFrame) -> Option<String> {
    const AECP_VENDOR_UNIQUE_RESPONSE: [u8; 2] = [0xfb, 0x07];
    const VENDOR_STATE_PROTOCOL: [u8; 6] = [0x00, 0x01, 0xf2, 0x00, 0x00, 0x01];
    const VENDOR_HEADER_LEN: usize = 28;
    const PROPERTY_HEADER_LEN: usize = 5;

    if frame.message_type != APP_AVDECC_FROM_APS
        || frame.payload.get(..2)? != AECP_VENDOR_UNIQUE_RESPONSE
        || frame.payload.get(22..28)? != VENDOR_STATE_PROTOCOL
    {
        return None;
    }
    let sequence = u16::from_be_bytes(frame.payload.get(20..22)?.try_into().ok()?);
    let property = frame.payload.get(VENDOR_HEADER_LEN..)?;
    let property_id = u16::from_be_bytes(property.get(..2)?.try_into().ok()?);
    let property_index = u16::from_be_bytes(property.get(2..4)?.try_into().ok()?);
    let value_size = usize::from(*property.get(4)?);
    let value = property.get(PROPERTY_HEADER_LEN..)?;
    if value.len() != value_size {
        return None;
    }
    let detail = vendor_state_detail_json(property_id, value);
    Some(format!(
        "{{\"protocol\":\"vendor_state\",\"vendor_protocol_id\":\"0001f2000001\",\"sequence\":{sequence},\"property_id\":\"{property_id:04x}\",\"property_index\":{property_index},\"value_size\":{value_size},\"value\":\"{}\"{detail}}}",
        hex_preview(value, value.len())
    ))
}

fn vendor_state_detail_json(property_id: u16, value: &[u8]) -> String {
    match (property_id, value) {
        (0x13b6, [mask]) => {
            let labels = [(0x01, "A"), (0x02, "B"), (0x04, "C")]
                .iter()
                .filter_map(|(bit, label)| (mask & bit != 0).then_some(format!("\"{label}\"")))
                .collect::<Vec<_>>()
                .join(",");
            format!(
                ",\"kind\":\"abc_selection\",\"mask\":{mask},\"enabled\":{},\"selected\":[{labels}]",
                *mask != 0
            )
        }
        (0x1394, [high, low]) => {
            let mask = u16::from_be_bytes([*high, *low]);
            let outputs = (0..12)
                .filter_map(|index| (mask & (1 << index) != 0).then_some((index + 1).to_string()))
                .collect::<Vec<_>>()
                .join(",");
            format!(",\"kind\":\"monitor_group\",\"mask\":{mask},\"line_outs\":[{outputs}]")
        }
        (0x139b, [state]) if *state <= 1 => {
            format!(",\"kind\":\"mute\",\"latched\":{}", state == &1)
        }
        (0x07d7, [state]) => {
            format!(",\"kind\":\"front_panel_event_unknown\",\"event_value\":{state}")
        }
        _ => String::new(),
    }
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_timestamped_adp_entity_available_metadata() {
        let mut payload = vec![0; 40];
        payload[0] = 0xfa;
        payload[1] = 0;
        payload[4..12].copy_from_slice(&0x0001_f2ff_fefe_b9e2u64.to_be_bytes());
        payload[36..40].copy_from_slice(&0x0004_9c6au32.to_be_bytes());
        let frame = AppFrame {
            version: 0,
            message_type: APP_AVDECC_FROM_APS,
            address: [0, 1, 2, 3, 4, 5],
            reserved: 0xffff,
            payload,
        };

        let json = observed_app_frame_json(&frame, 37);
        assert!(json.contains("\"received_ms\":37"));
        assert!(json.contains("\"protocol\":\"adp\""));
        assert!(json.contains("\"message_type\":\"entity_available\""));
        assert!(json.contains("\"entity_id\":\"0001f2fffefeb9e2\""));
        assert!(json.contains("\"available_index\":302186"));
    }

    #[test]
    fn formats_received_vendor_abc_selection_state() {
        let mut payload = vec![0xfb, 0x07, 0x00, 0x16];
        payload.extend_from_slice(&0x0001_f2ff_fefe_b9e2u64.to_be_bytes());
        payload.extend_from_slice(&0x0001_f261_fffe_b9e2u64.to_be_bytes());
        payload.extend_from_slice(&0x1234u16.to_be_bytes());
        payload.extend_from_slice(&[0x00, 0x01, 0xf2, 0x00, 0x00, 0x01]);
        payload.extend_from_slice(&[0x13, 0xb6, 0x00, 0x00, 0x01, 0x03]);
        let frame = AppFrame {
            version: 0,
            message_type: APP_AVDECC_FROM_APS,
            address: [0, 1, 2, 3, 4, 5],
            reserved: 0xffff,
            payload,
        };

        let json = observed_app_frame_json(&frame, 37);
        assert!(json.contains("\"protocol\":\"vendor_state\""));
        assert!(json.contains("\"sequence\":4660"));
        assert!(json.contains("\"property_id\":\"13b6\""));
        assert!(json.contains("\"kind\":\"abc_selection\""));
        assert!(json.contains("\"selected\":[\"A\",\"B\"]"));
    }

    #[test]
    fn rejects_vendor_state_with_a_truncated_value() {
        let frame = AppFrame {
            version: 0,
            message_type: APP_AVDECC_FROM_APS,
            address: [0, 1, 2, 3, 4, 5],
            reserved: 0xffff,
            payload: vec![
                0xfb, 0x07, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xf2,
                0, 0, 1, 0x13, 0xb6, 0, 0, 2, 3,
            ],
        };

        assert_eq!(avdecc_payload_json(&frame), "null");
    }
}
