use std::time::Duration;

use crate::device::json_escape;

use super::avdecc_aem::{
    configuration_descriptor_json, entity_descriptor_json, read_configuration_descriptor,
    read_descriptor, read_entity_descriptor, ConfigurationDescriptorResult, DescriptorResult,
    EntityDescriptorResult,
};
use super::avdecc_format::{app_frame_json, hex_preview};
use super::avdecc_transport::read_interface_mac;
use super::{decode_complete_v0_frames, AppFrame, AvdeccProxy, EntityIdResult, INITIAL_FRAME_WAIT};

pub(crate) struct AvdeccProbeResult {
    status: u16,
    reason: String,
    frames: Vec<AppFrame>,
    initial_data: Vec<u8>,
    entity_id: Option<u64>,
    entity_id_reserved: Option<u16>,
    entity_descriptor: Option<EntityDescriptorResult>,
    configuration_descriptor: Option<ConfigurationDescriptorResult>,
    descriptor: Option<DescriptorResult>,
}

pub(crate) struct DescriptorRead {
    pub(crate) target_entity_id: u64,
    pub(crate) descriptor_type: u16,
    pub(crate) descriptor_index: u16,
}

pub(crate) fn probe(
    host: &str,
    path: &str,
    interface: Option<&str>,
    target_entity_id: Option<u64>,
    configuration_target_entity_id: Option<u64>,
    descriptor_read: Option<DescriptorRead>,
    timeout: Duration,
) -> Result<AvdeccProbeResult, String> {
    let descriptor_requests = target_entity_id.is_some() as u8
        + configuration_target_entity_id.is_some() as u8
        + descriptor_read.is_some() as u8;
    if descriptor_requests > 1 {
        return Err("request only one descriptor per AVDECC probe".to_string());
    }
    if descriptor_requests > 0 && interface.is_none() {
        return Err("a descriptor read requires --request-entity-id".to_string());
    }
    let mut proxy = AvdeccProxy::connect(host, path, timeout)?;
    let preserve_initial_data = interface.is_some();
    let initial_data =
        proxy.read_available_for(timeout.min(INITIAL_FRAME_WAIT), preserve_initial_data)?;
    let initial_frames = if preserve_initial_data {
        Vec::new()
    } else {
        decode_complete_v0_frames(&initial_data)
    };
    let entity_id_result = if let Some(interface) = interface {
        proxy.request_entity_id(read_interface_mac(interface)?, timeout)?
    } else {
        EntityIdResult::default()
    };
    let entity_descriptor = if let Some(target_entity_id) = target_entity_id {
        let controller_entity_id = entity_id_result
            .entity_id
            .ok_or("AVDECC Proxy did not return an entity ID candidate")?;
        Some(read_entity_descriptor(
            &mut proxy,
            target_entity_id,
            controller_entity_id,
            timeout,
        )?)
    } else {
        None
    };
    let configuration_descriptor = if let Some(target_entity_id) = configuration_target_entity_id {
        let controller_entity_id = entity_id_result
            .entity_id
            .ok_or("AVDECC Proxy did not return an entity ID candidate")?;
        Some(read_configuration_descriptor(
            &mut proxy,
            target_entity_id,
            controller_entity_id,
            0,
            timeout,
        )?)
    } else {
        None
    };
    let descriptor = if let Some(request) = descriptor_read {
        let controller_entity_id = entity_id_result
            .entity_id
            .ok_or("AVDECC Proxy did not return an entity ID candidate")?;
        Some(read_descriptor(
            &mut proxy,
            request.target_entity_id,
            controller_entity_id,
            0,
            request.descriptor_type,
            request.descriptor_index,
            timeout,
        )?)
    } else {
        None
    };
    Ok(AvdeccProbeResult {
        status: proxy.response.status,
        reason: proxy.response.reason,
        frames: initial_frames
            .into_iter()
            .chain(entity_id_result.frames)
            .chain(
                entity_descriptor
                    .as_ref()
                    .into_iter()
                    .flat_map(|descriptor| descriptor.frames.iter().cloned()),
            )
            .chain(
                configuration_descriptor
                    .as_ref()
                    .into_iter()
                    .flat_map(|descriptor| descriptor.frames.iter().cloned()),
            )
            .chain(
                descriptor
                    .as_ref()
                    .into_iter()
                    .flat_map(|descriptor| descriptor.frames.iter().cloned()),
            )
            .collect(),
        initial_data,
        entity_id: entity_id_result.entity_id,
        entity_id_reserved: entity_id_result.reserved,
        entity_descriptor,
        configuration_descriptor,
        descriptor,
    })
}

pub(crate) fn write_probe_result(result: &AvdeccProbeResult) {
    let frames = result
        .frames
        .iter()
        .map(app_frame_json)
        .collect::<Vec<_>>()
        .join(",");
    println!(
        "{{\"status\":{},\"reason\":\"{}\",\"initial_bytes\":{},\"initial_preview\":\"{}\",\"entity_id_candidate\":{},\"entity_id_reserved\":{},\"entity_descriptor\":{},\"configuration_descriptor\":{},\"descriptor\":{},\"v0_frames\":[{}]}}",
        result.status,
        json_escape(&result.reason),
        result.initial_data.len(),
        hex_preview(&result.initial_data, 64),
        result
            .entity_id
            .map(|entity_id| format!("\"{entity_id:016x}\""))
            .unwrap_or_else(|| "null".to_string()),
        result
            .entity_id_reserved
            .map(|reserved| reserved.to_string())
            .unwrap_or_else(|| "null".to_string()),
        result
            .entity_descriptor
            .as_ref()
            .map(entity_descriptor_json)
            .unwrap_or_else(|| "null".to_string()),
        result
            .configuration_descriptor
            .as_ref()
            .map(configuration_descriptor_json)
            .unwrap_or_else(|| "null".to_string()),
        result
            .descriptor
            .as_ref()
            .map(super::avdecc_aem::descriptor_json)
            .unwrap_or_else(|| "null".to_string()),
        frames
    );
}
