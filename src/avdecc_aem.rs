use std::io::Write;
use std::time::{Duration, Instant};

use crate::device::json_escape;

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
const ENTITY_DESCRIPTOR_LEN: usize = 308;
const ENTITY_MODEL_ID_OFFSET: usize = 8;
const ENTITY_NAME_OFFSET: usize = 44;
const ENTITY_TEXT_LEN: usize = 64;
const FIRMWARE_VERSION_OFFSET: usize = 112;
const CONFIGURATIONS_COUNT_OFFSET: usize = 304;
const DESCRIPTOR_TYPE_INDEX_LEN: usize = 4;
const CONFIGURATION_DESCRIPTOR_HEADER_LEN: usize = 70;

pub(super) struct EntityDescriptorResult {
    pub(super) aem_status: u8,
    pub(super) descriptor: Vec<u8>,
    summary: Option<EntityDescriptorSummary>,
    pub(super) frames: Vec<AppFrame>,
}

struct EntityDescriptorSummary {
    model_id: u64,
    entity_name: String,
    firmware_version: String,
    configurations_count: u16,
    current_configuration: u16,
}

pub(super) struct ConfigurationDescriptorResult {
    pub(super) aem_status: u8,
    pub(super) descriptor: Vec<u8>,
    summary: Option<ConfigurationDescriptorSummary>,
    pub(super) frames: Vec<AppFrame>,
}

struct ConfigurationDescriptorSummary {
    object_name: String,
    descriptor_counts: Vec<DescriptorCount>,
}

struct DescriptorCount {
    descriptor_type: u16,
    count: u16,
}

pub(super) struct DescriptorResult {
    pub(super) aem_status: u8,
    pub(super) response_reserved: u16,
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
    Ok(EntityDescriptorResult {
        aem_status: result.aem_status,
        summary: parse_entity_descriptor_summary(&result.descriptor),
        descriptor: result.descriptor,
        frames: result.frames,
    })
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
    Ok(ConfigurationDescriptorResult {
        aem_status: result.aem_status,
        summary: parse_configuration_descriptor_summary(&result.descriptor),
        descriptor: result.descriptor,
        frames: result.frames,
    })
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
            return Ok(DescriptorResult {
                aem_status,
                response_reserved,
                descriptor,
                frames,
            });
        }
    }
}

pub(super) fn descriptor_json(result: &DescriptorResult) -> String {
    format!(
        "{{\"aem_status\":{},\"response_reserved\":{},\"bytes\":{},\"preview\":\"{}\"}}",
        result.aem_status,
        result.response_reserved,
        result.descriptor.len(),
        hex_preview(&result.descriptor, result.descriptor.len())
    )
}

pub(super) fn configuration_descriptor_json(result: &ConfigurationDescriptorResult) -> String {
    let summary = result
        .summary
        .as_ref()
        .map(configuration_descriptor_summary_json)
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"aem_status\":{},\"bytes\":{},\"preview\":\"{}\",\"summary\":{summary}}}",
        result.aem_status,
        result.descriptor.len(),
        hex_preview(&result.descriptor, 128)
    )
}

pub(super) fn entity_descriptor_json(result: &EntityDescriptorResult) -> String {
    let summary = result
        .summary
        .as_ref()
        .map(entity_descriptor_summary_json)
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"aem_status\":{},\"bytes\":{},\"preview\":\"{}\",\"summary\":{summary}}}",
        result.aem_status,
        result.descriptor.len(),
        hex_preview(&result.descriptor, 128)
    )
}

fn parse_entity_descriptor_summary(descriptor: &[u8]) -> Option<EntityDescriptorSummary> {
    if descriptor.len() < ENTITY_DESCRIPTOR_LEN {
        return None;
    }
    Some(EntityDescriptorSummary {
        model_id: u64::from_be_bytes(descriptor[ENTITY_MODEL_ID_OFFSET..16].try_into().ok()?),
        entity_name: descriptor_text(
            &descriptor[ENTITY_NAME_OFFSET..ENTITY_NAME_OFFSET + ENTITY_TEXT_LEN],
        ),
        firmware_version: descriptor_text(
            &descriptor[FIRMWARE_VERSION_OFFSET..FIRMWARE_VERSION_OFFSET + ENTITY_TEXT_LEN],
        ),
        configurations_count: u16::from_be_bytes(
            descriptor[CONFIGURATIONS_COUNT_OFFSET..CONFIGURATIONS_COUNT_OFFSET + 2]
                .try_into()
                .ok()?,
        ),
        current_configuration: u16::from_be_bytes(
            descriptor[CONFIGURATIONS_COUNT_OFFSET + 2..CONFIGURATIONS_COUNT_OFFSET + 4]
                .try_into()
                .ok()?,
        ),
    })
}

fn descriptor_text(bytes: &[u8]) -> String {
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..length]).into_owned()
}

fn entity_descriptor_summary_json(summary: &EntityDescriptorSummary) -> String {
    format!(
        "{{\"model_id\":\"{:016x}\",\"entity_name\":\"{}\",\"firmware_version\":\"{}\",\"configurations_count\":{},\"current_configuration\":{}}}",
        summary.model_id,
        json_escape(&summary.entity_name),
        json_escape(&summary.firmware_version),
        summary.configurations_count,
        summary.current_configuration
    )
}

fn parse_configuration_descriptor_summary(
    descriptor: &[u8],
) -> Option<ConfigurationDescriptorSummary> {
    if descriptor.len() < CONFIGURATION_DESCRIPTOR_HEADER_LEN {
        return None;
    }
    let descriptor_counts_count = u16::from_be_bytes(descriptor[66..68].try_into().ok()?) as usize;
    let descriptor_counts_offset = u16::from_be_bytes(descriptor[68..70].try_into().ok()?) as usize;
    let response_counts_offset = descriptor_counts_offset.checked_sub(DESCRIPTOR_TYPE_INDEX_LEN)?;
    if response_counts_offset < CONFIGURATION_DESCRIPTOR_HEADER_LEN {
        return None;
    }
    let expected_len = response_counts_offset.checked_add(descriptor_counts_count * 4)?;
    if descriptor.len() < expected_len {
        return None;
    }
    let descriptor_counts = (0..descriptor_counts_count)
        .map(|index| {
            let offset = response_counts_offset + index * 4;
            DescriptorCount {
                descriptor_type: u16::from_be_bytes([descriptor[offset], descriptor[offset + 1]]),
                count: u16::from_be_bytes([descriptor[offset + 2], descriptor[offset + 3]]),
            }
        })
        .collect();
    Some(ConfigurationDescriptorSummary {
        object_name: descriptor_text(&descriptor[..ENTITY_TEXT_LEN]),
        descriptor_counts,
    })
}

fn configuration_descriptor_summary_json(summary: &ConfigurationDescriptorSummary) -> String {
    let counts = summary
        .descriptor_counts
        .iter()
        .map(|entry| {
            format!(
                "{{\"descriptor_type\":\"0x{:04x}\",\"count\":{}}}",
                entry.descriptor_type, entry.count
            )
        })
        .collect::<Vec<_>>()
        .join(",");
    format!(
        "{{\"object_name\":\"{}\",\"descriptor_counts\":[{counts}]}}",
        json_escape(&summary.object_name)
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
    fn parses_fixed_entity_metadata() {
        let mut descriptor = vec![0; ENTITY_DESCRIPTOR_LEN];
        descriptor[..8].copy_from_slice(&0x0001_f2ff_fefe_b9e2u64.to_be_bytes());
        descriptor[8..16].copy_from_slice(&0x0001_f2ff_0000_0002u64.to_be_bytes());
        descriptor[44..47].copy_from_slice(b"848");
        descriptor[112..117].copy_from_slice(b"2.3.0");
        descriptor[304..306].copy_from_slice(&1u16.to_be_bytes());
        let summary = parse_entity_descriptor_summary(&descriptor).expect("valid descriptor");
        assert_eq!(summary.model_id, 0x0001_f2ff_0000_0002);
        assert_eq!(summary.entity_name, "848");
        assert_eq!(summary.firmware_version, "2.3.0");
        assert_eq!(summary.configurations_count, 1);
        assert_eq!(summary.current_configuration, 0);
    }

    #[test]
    fn parses_configuration_descriptor_counts() {
        let mut descriptor = vec![0; CONFIGURATION_DESCRIPTOR_HEADER_LEN + 8];
        descriptor[..7].copy_from_slice(b"Default");
        descriptor[66..68].copy_from_slice(&2u16.to_be_bytes());
        descriptor[68..70].copy_from_slice(&74u16.to_be_bytes());
        descriptor[70..74].copy_from_slice(&[0, 2, 0, 1]);
        descriptor[74..78].copy_from_slice(&[0, 20, 0, 12]);
        let summary =
            parse_configuration_descriptor_summary(&descriptor).expect("valid descriptor");
        assert_eq!(summary.object_name, "Default");
        assert_eq!(summary.descriptor_counts.len(), 2);
        assert_eq!(summary.descriptor_counts[0].descriptor_type, 2);
        assert_eq!(summary.descriptor_counts[1].count, 12);
    }

    #[test]
    fn derives_an_ethernet_address_from_a_standard_entity_id() {
        assert_eq!(
            eui48_from_entity_id(0x0001_f2ff_fefe_b9e2),
            Ok([0, 1, 0xf2, 0xfe, 0xb9, 0xe2])
        );
    }
}
