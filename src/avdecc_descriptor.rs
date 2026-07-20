use crate::device::json_escape;

use super::AppFrame;

const ENTITY_DESCRIPTOR_LEN: usize = 308;
const ENTITY_MODEL_ID_OFFSET: usize = 8;
const ENTITY_NAME_OFFSET: usize = 44;
const ENTITY_TEXT_LEN: usize = 64;
const FIRMWARE_VERSION_OFFSET: usize = 112;
const CONFIGURATIONS_COUNT_OFFSET: usize = 304;
const DESCRIPTOR_TYPE_INDEX_LEN: usize = 4;
const CONFIGURATION_DESCRIPTOR_HEADER_LEN: usize = 70;
const STRINGS_DESCRIPTOR_LEN: usize = 448;
const STRINGS_PER_DESCRIPTOR: usize = STRINGS_DESCRIPTOR_LEN / ENTITY_TEXT_LEN;
const AUDIO_UNIT_BODY_LEN: usize = 140;
const STREAM_PORT_BODY_LEN: usize = 16;

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

impl EntityDescriptorResult {
    pub(super) fn new(aem_status: u8, descriptor: Vec<u8>, frames: Vec<AppFrame>) -> Self {
        Self {
            aem_status,
            summary: parse_entity_descriptor_summary(&descriptor),
            descriptor,
            frames,
        }
    }
}

impl ConfigurationDescriptorResult {
    pub(super) fn new(aem_status: u8, descriptor: Vec<u8>, frames: Vec<AppFrame>) -> Self {
        Self {
            aem_status,
            summary: parse_configuration_descriptor_summary(&descriptor),
            descriptor,
            frames,
        }
    }
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

pub(super) fn strings_descriptor_json(descriptor: &[u8]) -> Option<String> {
    if descriptor.len() < STRINGS_DESCRIPTOR_LEN {
        return None;
    }
    let strings = descriptor[..STRINGS_DESCRIPTOR_LEN]
        .chunks_exact(ENTITY_TEXT_LEN)
        .take(STRINGS_PER_DESCRIPTOR)
        .map(|string| format!("\"{}\"", json_escape(&descriptor_text(string))))
        .collect::<Vec<_>>()
        .join(",");
    Some(format!("[{strings}]"))
}

pub(super) fn localized_description(descriptor: &[u8]) -> Option<u16> {
    descriptor
        .get(ENTITY_TEXT_LEN..ENTITY_TEXT_LEN + 2)
        .map(|bytes| u16::from_be_bytes([bytes[0], bytes[1]]))
}

pub(super) fn audio_unit_topology_json(descriptor: &[u8]) -> Option<String> {
    if descriptor.len() < AUDIO_UNIT_BODY_LEN {
        return None;
    }
    let ranges = [
        ("stream_input_ports", 68, 70),
        ("stream_output_ports", 72, 74),
        ("external_input_ports", 76, 78),
        ("external_output_ports", 80, 82),
        ("internal_input_ports", 84, 86),
        ("internal_output_ports", 88, 90),
        ("controls", 92, 94),
        ("signal_selectors", 96, 98),
        ("mixers", 100, 102),
        ("matrices", 104, 106),
    ]
    .into_iter()
    .map(|(name, count_offset, base_offset)| {
        format!(
            "\"{name}\":{{\"count\":{},\"base\":{}}}",
            read_u16(descriptor, count_offset),
            read_u16(descriptor, base_offset)
        )
    })
    .collect::<Vec<_>>()
    .join(",");
    Some(format!("{{{ranges}}}"))
}

pub(super) fn stream_port_topology_json(descriptor: &[u8]) -> Option<String> {
    if descriptor.len() < STREAM_PORT_BODY_LEN {
        return None;
    }
    Some(format!(
        "{{\"controls\":{{\"count\":{},\"base\":{}}},\"clusters\":{{\"count\":{},\"base\":{}}},\"maps\":{{\"count\":{},\"base\":{}}}}}",
        read_u16(descriptor, 4),
        read_u16(descriptor, 6),
        read_u16(descriptor, 8),
        read_u16(descriptor, 10),
        read_u16(descriptor, 12),
        read_u16(descriptor, 14),
    ))
}

pub(super) fn audio_cluster_topology_json(descriptor: &[u8]) -> Option<String> {
    if descriptor.len() < 83 {
        return None;
    }
    Some(format!(
        "{{\"signal_type\":\"0x{:04x}\",\"signal_index\":{},\"signal_output\":{},\"channels\":{},\"format\":{}}}",
        read_u16(descriptor, 66),
        read_u16(descriptor, 68),
        read_u16(descriptor, 70),
        read_u16(descriptor, 80),
        descriptor[82],
    ))
}

pub(super) fn control_topology_json(descriptor: &[u8]) -> Option<String> {
    if descriptor.len() < 100 {
        return None;
    }
    let control_type = u64::from_be_bytes(descriptor[78..86].try_into().ok()?);
    Some(format!(
        "{{\"value_type\":\"0x{:04x}\",\"control_type\":\"0x{control_type:016x}\",\"values_offset\":{},\"number_of_values\":{},\"signal_type\":\"0x{:04x}\",\"signal_index\":{},\"signal_output\":{}}}",
        read_u16(descriptor, 76),
        read_u16(descriptor, 90),
        read_u16(descriptor, 92),
        read_u16(descriptor, 94),
        read_u16(descriptor, 96),
        read_u16(descriptor, 98),
    ))
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

fn descriptor_text(bytes: &[u8]) -> String {
    let length = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..length]).into_owned()
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_be_bytes([bytes[offset], bytes[offset + 1]])
}

fn hex_preview(bytes: &[u8], maximum: usize) -> String {
    bytes
        .iter()
        .take(maximum)
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

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
    fn reads_all_seven_strings() {
        let mut descriptor = vec![0; STRINGS_DESCRIPTOR_LEN];
        descriptor[..4].copy_from_slice(b"MOTU");
        descriptor[64..67].copy_from_slice(b"848");
        assert_eq!(
            strings_descriptor_json(&descriptor),
            Some("[\"MOTU\",\"848\",\"\",\"\",\"\",\"\",\"\"]".to_string())
        );
    }

    #[test]
    fn reads_a_localized_description_after_an_object_name() {
        let mut descriptor = vec![0; ENTITY_TEXT_LEN + 2];
        descriptor[64..66].copy_from_slice(&73u16.to_be_bytes());
        assert_eq!(localized_description(&descriptor), Some(73));
    }

    #[test]
    fn parses_audio_unit_child_ranges() {
        let mut descriptor = vec![0; AUDIO_UNIT_BODY_LEN];
        descriptor[68..72].copy_from_slice(&[0, 1, 0, 2]);
        descriptor[92..96].copy_from_slice(&[0, 3, 0, 4]);
        let summary = audio_unit_topology_json(&descriptor).expect("valid audio unit");
        assert!(summary.contains("\"stream_input_ports\":{\"count\":1,\"base\":2}"));
        assert!(summary.contains("\"controls\":{\"count\":3,\"base\":4}"));
    }

    #[test]
    fn parses_stream_port_child_ranges() {
        let mut descriptor = vec![0; STREAM_PORT_BODY_LEN];
        descriptor[8..12].copy_from_slice(&[0, 2, 0, 3]);
        let summary = stream_port_topology_json(&descriptor).expect("valid stream port");
        assert!(summary.contains("\"clusters\":{\"count\":2,\"base\":3}"));
    }

    #[test]
    fn parses_audio_cluster_signal_details() {
        let mut descriptor = vec![0; 83];
        descriptor[66..72].copy_from_slice(&[0, 20, 0, 23, 0, 1]);
        descriptor[80..82].copy_from_slice(&2u16.to_be_bytes());
        descriptor[82] = 1;
        let summary = audio_cluster_topology_json(&descriptor).expect("valid audio cluster");
        assert!(summary.contains("\"signal_index\":23"));
        assert!(summary.contains("\"channels\":2"));
    }

    #[test]
    fn parses_control_details() {
        let mut descriptor = vec![0; 100];
        descriptor[76..78].copy_from_slice(&1u16.to_be_bytes());
        descriptor[78..86].copy_from_slice(&0x90e0_f000_0000_0001u64.to_be_bytes());
        descriptor[92..94].copy_from_slice(&2u16.to_be_bytes());
        let summary = control_topology_json(&descriptor).expect("valid control");
        assert!(summary.contains("\"control_type\":\"0x90e0f00000000001\""));
        assert!(summary.contains("\"number_of_values\":2"));
    }
}
