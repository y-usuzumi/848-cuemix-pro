use std::io::Write;
use std::time::{Duration, Instant};

use super::avdecc_descriptor::{
    audio_cluster_topology_json, audio_unit_topology_json, control_topology_json,
    localized_description, minimum_descriptor_body_len, stream_port_topology_json,
    strings_descriptor_json, ConfigurationDescriptorResult, EntityDescriptorResult,
};
use super::{hex_preview, AppFrame, AvdeccProxy, APP_AVDECC_FROM_APC, APP_AVDECC_FROM_APS};

const AECP_SUBTYPE: u8 = 0xfb;
const AEM_COMMAND: u8 = 0x00;
const AEM_RESPONSE: u8 = 0x01;
const AEM_READ_DESCRIPTOR: u16 = 0x0004;
const ENTITY_DESCRIPTOR: u16 = 0x0000;
const CONFIGURATION_DESCRIPTOR: u16 = 0x0001;
const ENTITY_DESCRIPTOR_REQUEST_LEN: u16 = 20;
const AECP_COMMON_LEN: usize = 24;
const READ_DESCRIPTOR_RESPONSE_LEN: usize = 32;

pub(super) struct DescriptorResult {
    pub(super) aem_status: u8,
    pub(super) response_reserved: u16,
    descriptor_type: u16,
    descriptor_index: u16,
    pub(super) descriptor: Vec<u8>,
    pub(super) frames: Vec<AppFrame>,
}

pub(super) fn read_entity_descriptor(
    proxy: &mut AvdeccProxy,
    target_entity_id: u64,
    controller_entity_id: u64,
    timeout: Duration,
) -> Result<EntityDescriptorResult, String> {
    let result = read_descriptor(
        proxy,
        target_entity_id,
        controller_entity_id,
        0,
        ENTITY_DESCRIPTOR,
        0,
        timeout,
    )?;
    if result.aem_status == 0
        && (result.descriptor.len() < 8 || result.descriptor[..8] != target_entity_id.to_be_bytes())
    {
        return Err(format!(
            "invalid successful ENTITY descriptor response: {}",
            hex_preview(&result.descriptor, 32)
        ));
    }
    Ok(EntityDescriptorResult::new(
        result.aem_status,
        result.descriptor,
        result.frames,
    ))
}

pub(super) fn read_configuration_descriptor(
    proxy: &mut AvdeccProxy,
    target_entity_id: u64,
    controller_entity_id: u64,
    configuration_index: u16,
    timeout: Duration,
) -> Result<ConfigurationDescriptorResult, String> {
    let result = read_descriptor(
        proxy,
        target_entity_id,
        controller_entity_id,
        configuration_index,
        CONFIGURATION_DESCRIPTOR,
        0,
        timeout,
    )?;
    Ok(ConfigurationDescriptorResult::new(
        result.aem_status,
        result.descriptor,
        result.frames,
    ))
}

pub(super) fn read_descriptor(
    proxy: &mut AvdeccProxy,
    target_entity_id: u64,
    controller_entity_id: u64,
    configuration_index: u16,
    descriptor_type: u16,
    descriptor_index: u16,
    timeout: Duration,
) -> Result<DescriptorResult, String> {
    let target_mac = eui48_from_entity_id(target_entity_id)?;
    let sequence_id = 1;
    let request = AppFrame {
        version: 0,
        message_type: APP_AVDECC_FROM_APC,
        address: target_mac,
        reserved: 0,
        payload: aem_read_descriptor_command(
            target_entity_id,
            controller_entity_id,
            sequence_id,
            configuration_index,
            descriptor_type,
            descriptor_index,
        ),
    };
    proxy
        .stream
        .write_all(&request.encode()?)
        .map_err(|error| format!("write AVDECC Proxy READ_DESCRIPTOR failed: {error}"))?;

    let deadline = Instant::now() + timeout;
    let mut frames: Vec<AppFrame> = Vec::new();
    loop {
        let Some(frame) = proxy.read_frame_until(deadline)? else {
            let preview = frames
                .last()
                .map(|frame| hex_preview(&frame.payload, 32))
                .unwrap_or_else(|| "no AVDECC frames received".to_string());
            return Err(format!(
                "timed out waiting for AVDECC Proxy READ_DESCRIPTOR response: {preview}"
            ));
        };
        let result = if frame.message_type == APP_AVDECC_FROM_APS && frame.address == target_mac {
            parse_read_descriptor_response(
                &frame.payload,
                target_entity_id,
                controller_entity_id,
                sequence_id,
                configuration_index,
                descriptor_type,
                descriptor_index,
            )?
        } else {
            None
        };
        frames.push(frame);
        if let Some((aem_status, response_reserved, descriptor)) = result {
            validate_successful_descriptor_body(
                descriptor_type,
                descriptor_index,
                aem_status,
                &descriptor,
            )?;
            return Ok(DescriptorResult {
                aem_status,
                response_reserved,
                descriptor_type,
                descriptor_index,
                descriptor,
                frames,
            });
        }
    }
}

fn validate_successful_descriptor_body(
    descriptor_type: u16,
    descriptor_index: u16,
    aem_status: u8,
    descriptor: &[u8],
) -> Result<(), String> {
    let minimum = minimum_descriptor_body_len(descriptor_type);
    if aem_status == 0 && descriptor.len() < minimum {
        return Err(format!(
            "successful descriptor 0x{descriptor_type:04x}:{descriptor_index} is only {} bytes; expected at least {minimum}",
            descriptor.len()
        ));
    }
    Ok(())
}

pub(super) fn descriptor_json(result: &DescriptorResult) -> String {
    let strings = if result.descriptor_type == 0x000d {
        strings_descriptor_json(&result.descriptor).unwrap_or_else(|| "null".to_string())
    } else {
        "null".to_string()
    };
    let has_localized_description = matches!(
        result.descriptor_type,
        0x0002 | 0x0005 | 0x0006 | 0x0014 | 0x001a | 0x001d
    );
    let localized_description = has_localized_description
        .then(|| localized_description(&result.descriptor))
        .flatten()
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    let topology = match result.descriptor_type {
        0x0002 => audio_unit_topology_json(&result.descriptor),
        0x000e | 0x000f => stream_port_topology_json(&result.descriptor),
        0x0014 => audio_cluster_topology_json(&result.descriptor),
        0x001a => control_topology_json(&result.descriptor),
        _ => None,
    }
    .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"aem_status\":{},\"response_reserved\":{},\"descriptor_type\":\"0x{:04x}\",\"descriptor_index\":{},\"localized_description\":{},\"topology\":{topology},\"bytes\":{},\"preview\":\"{}\",\"strings\":{strings}}}",
        result.aem_status,
        result.response_reserved,
        result.descriptor_type,
        result.descriptor_index,
        localized_description,
        result.descriptor.len(),
        hex_preview(&result.descriptor, result.descriptor.len())
    )
}

fn aem_read_descriptor_command(
    target_entity_id: u64,
    controller_entity_id: u64,
    sequence_id: u16,
    configuration_index: u16,
    descriptor_type: u16,
    descriptor_index: u16,
) -> Vec<u8> {
    let mut bytes = Vec::with_capacity(READ_DESCRIPTOR_RESPONSE_LEN);
    bytes.push(AECP_SUBTYPE);
    bytes.push(AEM_COMMAND);
    bytes.extend_from_slice(&ENTITY_DESCRIPTOR_REQUEST_LEN.to_be_bytes());
    bytes.extend_from_slice(&target_entity_id.to_be_bytes());
    bytes.extend_from_slice(&controller_entity_id.to_be_bytes());
    bytes.extend_from_slice(&sequence_id.to_be_bytes());
    bytes.extend_from_slice(&AEM_READ_DESCRIPTOR.to_be_bytes());
    bytes.extend_from_slice(&configuration_index.to_be_bytes());
    bytes.extend_from_slice(&0u16.to_be_bytes());
    bytes.extend_from_slice(&descriptor_type.to_be_bytes());
    bytes.extend_from_slice(&descriptor_index.to_be_bytes());
    bytes
}

fn parse_read_descriptor_response(
    bytes: &[u8],
    target_entity_id: u64,
    controller_entity_id: u64,
    sequence_id: u16,
    configuration_index: u16,
    descriptor_type: u16,
    descriptor_index: u16,
) -> Result<Option<(u8, u16, Vec<u8>)>, String> {
    if bytes.len() < READ_DESCRIPTOR_RESPONSE_LEN
        || bytes[0] != AECP_SUBTYPE
        || bytes[1] != AEM_RESPONSE
    {
        return Ok(None);
    }
    let status_and_length = u16::from_be_bytes([bytes[2], bytes[3]]);
    let aem_status = (status_and_length >> 11) as u8;
    let control_data_length = (status_and_length & 0x07ff) as usize;
    if bytes.len() != control_data_length + 12 {
        return Err("invalid AECP READ_DESCRIPTOR control data length".to_string());
    }
    if bytes[4..12] != target_entity_id.to_be_bytes()
        || bytes[12..20] != controller_entity_id.to_be_bytes()
        || bytes[20..22] != sequence_id.to_be_bytes()
        || bytes[22..24] != AEM_READ_DESCRIPTOR.to_be_bytes()
        || bytes[24..26] != configuration_index.to_be_bytes()
        || bytes[28..30] != descriptor_type.to_be_bytes()
        || bytes[30..32] != descriptor_index.to_be_bytes()
    {
        return Ok(None);
    }
    let response_reserved = u16::from_be_bytes([bytes[26], bytes[27]]);
    let descriptor = bytes[AECP_COMMON_LEN + 8..].to_vec();
    Ok(Some((aem_status, response_reserved, descriptor)))
}

fn eui48_from_entity_id(entity_id: u64) -> Result<[u8; 6], String> {
    let bytes = entity_id.to_be_bytes();
    if bytes[3..5] != [0xff, 0xfe] {
        return Err("target entity ID does not contain a derivable EUI-48 address".to_string());
    }
    Ok([bytes[0], bytes[1], bytes[2], bytes[5], bytes[6], bytes[7]])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn encodes_a_read_only_entity_descriptor_request() {
        let request = aem_read_descriptor_command(
            0x0001_f2ff_fefe_b9e2,
            0x0001_f210_fffe_b9e2,
            1,
            0,
            ENTITY_DESCRIPTOR,
            0,
        );
        assert_eq!(request.len(), READ_DESCRIPTOR_RESPONSE_LEN);
        assert_eq!(&request[..4], &[AECP_SUBTYPE, AEM_COMMAND, 0, 20]);
        assert_eq!(&request[22..24], &AEM_READ_DESCRIPTOR.to_be_bytes());
        assert_eq!(&request[24..], &[0, 0, 0, 0, 0, 0, 0, 0]);
    }

    #[test]
    fn parses_a_successful_entity_descriptor_response() {
        let target = 0x0001_f2ff_fefe_b9e2;
        let controller = 0x0001_f210_fffe_b9e2;
        let mut response =
            aem_read_descriptor_command(target, controller, 1, 0, ENTITY_DESCRIPTOR, 0);
        response[1] = AEM_RESPONSE;
        response[26..28].copy_from_slice(&0x6370u16.to_be_bytes());
        response.extend_from_slice(&target.to_be_bytes());
        response[2..4].copy_from_slice(&28u16.to_be_bytes());
        assert_eq!(
            parse_read_descriptor_response(
                &response,
                target,
                controller,
                1,
                0,
                ENTITY_DESCRIPTOR,
                0,
            ),
            Ok(Some((0, 0x6370, target.to_be_bytes().to_vec())))
        );
    }

    #[test]
    fn accepts_a_read_descriptor_error_without_descriptor_data() {
        let target = 0x0001_f2ff_fefe_b9e2;
        let controller = 0x0001_f210_fffe_b9e2;
        let mut response =
            aem_read_descriptor_command(target, controller, 1, 0, ENTITY_DESCRIPTOR, 0);
        response[1] = AEM_RESPONSE;
        response[2..4].copy_from_slice(&(1u16 << 11 | 20).to_be_bytes());
        assert_eq!(
            parse_read_descriptor_response(
                &response,
                target,
                controller,
                1,
                0,
                ENTITY_DESCRIPTOR,
                0,
            ),
            Ok(Some((1, 0, Vec::new())))
        );
    }

    #[test]
    fn rejects_successful_truncated_descriptors() {
        assert!(validate_successful_descriptor_body(ENTITY_DESCRIPTOR, 0, 0, &[]).is_err());
        assert!(validate_successful_descriptor_body(0x00fe, 0, 0, &[]).is_err());
        assert!(validate_successful_descriptor_body(ENTITY_DESCRIPTOR, 0, 1, &[]).is_ok());
    }

    #[test]
    fn rejects_mismatched_read_descriptor_response_fields() {
        let target = 0x0001_f2ff_fefe_b9e2;
        let controller = 0x0001_f210_fffe_b9e2;
        let mut response =
            aem_read_descriptor_command(target, controller, 1, 0, ENTITY_DESCRIPTOR, 0);
        response[1] = AEM_RESPONSE;
        response.extend_from_slice(&target.to_be_bytes());
        response[2..4].copy_from_slice(&28u16.to_be_bytes());
        for offset in [4, 12, 20, 22, 24, 28, 30] {
            let mut mismatched = response.clone();
            mismatched[offset] ^= 1;
            assert_eq!(
                parse_read_descriptor_response(
                    &mismatched,
                    target,
                    controller,
                    1,
                    0,
                    ENTITY_DESCRIPTOR,
                    0,
                ),
                Ok(None)
            );
        }
    }

    #[test]
    fn rejects_malformed_read_descriptor_response_length() {
        let target = 0x0001_f2ff_fefe_b9e2;
        let controller = 0x0001_f210_fffe_b9e2;
        let mut response =
            aem_read_descriptor_command(target, controller, 1, 0, ENTITY_DESCRIPTOR, 0);
        response[1] = AEM_RESPONSE;
        response[2..4].copy_from_slice(&19u16.to_be_bytes());
        assert!(parse_read_descriptor_response(
            &response,
            target,
            controller,
            1,
            0,
            ENTITY_DESCRIPTOR,
            0,
        )
        .is_err());
    }

    #[test]
    fn derives_an_ethernet_address_from_a_standard_entity_id() {
        assert_eq!(
            eui48_from_entity_id(0x0001_f2ff_fefe_b9e2),
            Ok([0, 1, 0xf2, 0xfe, 0xb9, 0xe2])
        );
    }
}
