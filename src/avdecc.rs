use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::sync::{mpsc, Arc, Condvar, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use crate::device::{parse_http_response, HttpResponse};

#[path = "avdecc_format.rs"]
mod avdecc_format;

use avdecc_format::hex_preview;

#[path = "avdecc_transport.rs"]
mod avdecc_transport;

use avdecc_transport::{connect_with_timeout, parse_proxy_address, validate_proxy_path};

#[path = "avdecc_aem.rs"]
mod avdecc_aem;

#[path = "avdecc_descriptor.rs"]
mod avdecc_descriptor;

#[path = "avdecc_probe.rs"]
mod avdecc_probe;

#[path = "avdecc_console.rs"]
mod avdecc_console;
pub(crate) use avdecc_console::{parse_changes, write_console};

pub(crate) use avdecc_probe::{probe, write_probe_result, DescriptorRead};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MixerFader {
    MainHost11To12,
    HeadphoneHost11To12,
    MainLineIn5To6,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum MixerLevel {
    Minus12,
    Minus60,
}

#[derive(Clone, Debug, Default)]
pub(crate) struct MixerMeters {
    pub(crate) records: Vec<MixerMeterRecord>,
    pub(crate) updated_at: Option<Instant>,
    pub(crate) error: Option<String>,
}

#[derive(Clone, Debug)]
pub(crate) struct MixerMeterFeedSnapshot {
    pub(crate) revision: u64,
    pub(crate) meters: MixerMeters,
    pub(crate) closed: bool,
}

#[derive(Debug, Default)]
struct MixerMeterFeedState {
    revision: u64,
    meters: MixerMeters,
    closed: bool,
}

#[derive(Debug, Default)]
pub(crate) struct MixerMeterFeed {
    state: Mutex<MixerMeterFeedState>,
    changed: Condvar,
}

impl MixerMeterFeed {
    pub(crate) fn snapshot(&self) -> Result<MixerMeterFeedSnapshot, String> {
        self.state
            .lock()
            .map(|state| MixerMeterFeedSnapshot {
                revision: state.revision,
                meters: state.meters.clone(),
                closed: state.closed,
            })
            .map_err(|_| "meter worker data is unavailable".to_string())
    }

    pub(crate) fn wait_after(
        &self,
        revision: u64,
        timeout: Duration,
    ) -> Result<Option<MixerMeterFeedSnapshot>, String> {
        let state = self
            .state
            .lock()
            .map_err(|_| "meter worker data is unavailable".to_string())?;
        let (state, wait) = self
            .changed
            .wait_timeout_while(state, timeout, |state| {
                state.revision == revision && !state.closed
            })
            .map_err(|_| "meter worker data is unavailable".to_string())?;
        if wait.timed_out() && state.revision == revision && !state.closed {
            return Ok(None);
        }
        Ok(Some(MixerMeterFeedSnapshot {
            revision: state.revision,
            meters: state.meters.clone(),
            closed: state.closed,
        }))
    }

    fn publish(&self, records: Vec<MixerMeterRecord>, error: Option<String>) {
        if let Ok(mut state) = self.state.lock() {
            if !records.is_empty() {
                state.meters.records = records;
                state.meters.updated_at = Some(Instant::now());
            }
            state.meters.error = error;
            state.revision = state.revision.wrapping_add(1);
            self.changed.notify_all();
        }
    }

    fn close(&self) {
        if let Ok(mut state) = self.state.lock() {
            state.closed = true;
            self.changed.notify_all();
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct MixerMeterRecord {
    pub(crate) property_id: u16,
    pub(crate) index: u8,
    pub(crate) values: Vec<u16>,
}

impl MixerMeterRecord {
    pub(crate) fn channels(&self) -> Vec<u8> {
        self.values
            .iter()
            .flat_map(|value| value.to_be_bytes())
            .collect()
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct HeadphoneOutput {
    pub(crate) channel_indices: [u16; 2],
    pub(crate) attenuation: [u8; 2],
    pub(crate) meter_path: Option<MeterPath>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineOutput {
    pub(crate) channel_index: u16,
    pub(crate) attenuation: u8,
    pub(crate) meter_path: Option<MeterPath>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MeterPath {
    pub(crate) property_id: u16,
    pub(crate) record_index: u8,
    pub(crate) channel_index: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct LineInput {
    pub(crate) channel_index: u16,
    pub(crate) gain_db: u8,
    pub(crate) phase_inverted: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct OutputInventory {
    pub(crate) line_outputs: Vec<LineOutput>,
    pub(crate) headphone_outputs: Vec<HeadphoneOutput>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum OutputTrim {
    NegativeInfinity,
    Decibels(i16),
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct VendorStateRecord {
    property_id: u16,
    property_index: u16,
    value: Vec<u8>,
}

const LINE_OUTPUT_TRIM_PROPERTY: u16 = 0x1388;
const HEADPHONE_TRIM_PROPERTY: u16 = 0x13b7;
const LINE_OUTPUT_METER_PATH_PROPERTY: u16 = 0x93ac;
const HEADPHONE_METER_PATH_PROPERTY: u16 = 0x13b4;
const LINE_INPUT_GAIN_PROPERTY: u16 = 0x13b2;
const LINE_INPUT_PHASE_PROPERTY: u16 = 0x13b3;
const LINE_INPUT_GAIN_MAX_DB: u8 = 20;

impl OutputTrim {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        if value == "-inf" {
            return Ok(Self::NegativeInfinity);
        }
        match value.parse::<i16>() {
            Ok(decibels) if (-99..=0).contains(&decibels) => Ok(Self::Decibels(decibels)),
            _ => Err("output trim must be -inf or an integer between -99 and 0 dB".to_string()),
        }
    }

    fn attenuation(self) -> u8 {
        match self {
            Self::NegativeInfinity => 100,
            Self::Decibels(decibels) => {
                u8::try_from(-decibels).expect("validated headphone dB fits in one byte")
            }
        }
    }
}

impl MixerFader {
    pub(crate) fn parse(bus: &str, source: &str) -> Result<Self, String> {
        match (bus, source) {
            ("main-1-2", "host-11-12") => Ok(Self::MainHost11To12),
            ("headphone-mix", "host-11-12") => Ok(Self::HeadphoneHost11To12),
            ("main-1-2", "line-in-5-6") => Ok(Self::MainLineIn5To6),
            _ => Err("that mixer bus/source pair has not been capture-validated yet".to_string()),
        }
    }

    fn property_and_index(self) -> (u16, u16) {
        match self {
            Self::MainHost11To12 => (0x841a, 0x0a00),
            Self::HeadphoneHost11To12 => (0x83f8, 0x0a00),
            Self::MainLineIn5To6 => (0x841a, 0x1000),
        }
    }

    pub(crate) fn meter_slot(self) -> (u16, u8, usize) {
        // Protocol ...:04 exposes 32 stereo-pair samples for the 64-channel
        // Mix In bank as 0x13ad:0. The paired source map places Host 11-12 at
        // slot 5 and Analog (Line) 5-6 at slot 10. The vendor fader index for
        // the latter is a source-family selector, not its Mix In slot number.
        match self {
            Self::MainHost11To12 | Self::HeadphoneHost11To12 => (0x13ad, 0, 5),
            Self::MainLineIn5To6 => (0x13ad, 0, 10),
        }
    }
}

impl MixerLevel {
    pub(crate) fn parse(value: &str) -> Result<Self, String> {
        match value {
            "-12" => Ok(Self::Minus12),
            "-60" => Ok(Self::Minus60),
            _ => {
                Err("only capture-validated mixer levels -12 and -60 dB are available".to_string())
            }
        }
    }

    fn encoded_value(self) -> u32 {
        match self {
            Self::Minus12 => 0x0040_4de6,
            Self::Minus60 => 0x0000_4189,
        }
    }
}

/// Sends one user-requested fader update through the capture-validated vendor
/// session. This is intentionally limited to the exact bus/source/value triples
/// observed in CueMix Pro captures; it performs no automatic hardware test.
pub(crate) fn set_mixer_fader(
    host: &str,
    target_entity_id: u64,
    fader: MixerFader,
    level: MixerLevel,
    timeout: Duration,
) -> Result<(), String> {
    // The 848 occasionally rejects a just-reopened CueMix session. Repeating
    // the same explicitly requested fader value is idempotent, so make one
    // fresh-session retry rather than exposing a spurious UI failure.
    const RETRY_DELAY: Duration = Duration::from_millis(150);
    match set_mixer_fader_once(host, target_entity_id, fader, level, timeout) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            thread::sleep(RETRY_DELAY);
            set_mixer_fader_once(host, target_entity_id, fader, level, timeout).map_err(
                |retry_error| {
                    format!(
                        "CueMix fader write failed after one fresh-session retry; first attempt: {first_error}; retry: {retry_error}"
                    )
                },
            )
        }
    }
}

fn set_mixer_fader_once(
    host: &str,
    target_entity_id: u64,
    fader: MixerFader,
    level: MixerLevel,
    timeout: Duration,
) -> Result<(), String> {
    const CUE_MIX_PROXY_ADDRESS: [u8; 6] = [0x01, 0x00, 0x00, 0x00, 0x01, 0x00];
    let mut proxy = AvdeccProxy::connect(host, "/", timeout)?;
    let controller_entity_id = proxy
        .request_entity_id(CUE_MIX_PROXY_ADDRESS, timeout)?
        .entity_id
        .ok_or("AVDECC Proxy did not return a controller identity")?;
    let (next_sequence, _) =
        proxy.start_vendor_state(target_entity_id, controller_entity_id, timeout)?;
    let (property, index) = fader.property_and_index();
    let mut data = Vec::with_capacity(9);
    data.extend_from_slice(&property.to_be_bytes());
    data.extend_from_slice(&index.to_be_bytes());
    data.push(4);
    data.extend_from_slice(&level.encoded_value().to_be_bytes());
    proxy.vendor_request(
        target_entity_id,
        controller_entity_id,
        next_sequence,
        [0x00, 0x01, 0xf2, 0x00, 0x00, 0x03],
        &data,
        timeout,
    )?;
    Ok(())
}

pub(crate) struct VendorSnapshot(Vec<VendorStateRecord>);

impl VendorSnapshot {
    pub(crate) fn console(self) -> Result<avdecc_console::ConsoleState, String> {
        avdecc_console::ConsoleState::from_records(self.0)
    }

    pub(crate) fn outputs(&self) -> Result<OutputInventory, String> {
        Ok(OutputInventory {
            line_outputs: line_outputs_from_state(&self.0)?,
            headphone_outputs: headphone_outputs_from_state(&self.0)?,
        })
    }

    pub(crate) fn line_inputs(&self) -> Result<Vec<LineInput>, String> {
        line_inputs_from_state(&self.0)
    }
}

pub(crate) struct VendorSnapshotRequest {
    pub(crate) deadline: Instant,
    pub(crate) reply: mpsc::Sender<Result<VendorSnapshot, String>>,
}

pub(crate) struct MixerMeterWorker {
    pub(crate) stop_sender: mpsc::Sender<mpsc::Sender<()>>,
    pub(crate) state_sender: mpsc::Sender<VendorSnapshotRequest>,
    pub(crate) meters: Arc<MixerMeterFeed>,
}

/// Sends one explicitly requested line-input polarity value after rediscovering
/// the target's input indices in a fresh vendor-state session.
pub(crate) fn set_line_input_phase(
    host: &str,
    target_entity_id: u64,
    input_index: usize,
    phase_inverted: bool,
    timeout: Duration,
) -> Result<(), String> {
    set_line_input_property(
        host,
        target_entity_id,
        input_index,
        LINE_INPUT_PHASE_PROPERTY,
        u8::from(phase_inverted),
        "line-input polarity",
        timeout,
    )
}

fn set_line_input_property(
    host: &str,
    target_entity_id: u64,
    input_index: usize,
    property_id: u16,
    value: u8,
    description: &str,
    timeout: Duration,
) -> Result<(), String> {
    const RETRY_DELAY: Duration = Duration::from_millis(150);
    match set_line_input_property_once(
        host,
        target_entity_id,
        input_index,
        property_id,
        value,
        timeout,
    ) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            thread::sleep(RETRY_DELAY);
            set_line_input_property_once(
                host,
                target_entity_id,
                input_index,
                property_id,
                value,
                timeout,
            )
            .map_err(|retry_error| {
                format!(
                    "{description} write failed after one fresh-session retry; first attempt: {first_error}; retry: {retry_error}"
                )
            })
        }
    }
}

fn set_line_input_property_once(
    host: &str,
    target_entity_id: u64,
    input_index: usize,
    property_id: u16,
    value: u8,
    timeout: Duration,
) -> Result<(), String> {
    const CUE_MIX_PROXY_ADDRESS: [u8; 6] = [0x01, 0x00, 0x00, 0x00, 0x01, 0x00];
    const PROPERTY_WRITE_PROTOCOL: [u8; 6] = [0x00, 0x01, 0xf2, 0x00, 0x00, 0x03];
    let mut proxy = AvdeccProxy::connect(host, "/", timeout)?;
    let controller_entity_id = proxy
        .request_entity_id(CUE_MIX_PROXY_ADDRESS, timeout)?
        .entity_id
        .ok_or("AVDECC Proxy did not return a controller identity")?;
    let (next_sequence, state) =
        proxy.start_vendor_state(target_entity_id, controller_entity_id, timeout)?;
    let inputs = line_inputs_from_state(&state)?;
    let input = inputs
        .get(input_index)
        .ok_or("that line input was not advertised by the device")?;
    let data = one_byte_property_payload(property_id, input.channel_index, value);
    proxy.vendor_request(
        target_entity_id,
        controller_entity_id,
        next_sequence,
        PROPERTY_WRITE_PROTOCOL,
        &data,
        timeout,
    )?;
    Ok(())
}

/// Sends one explicitly requested linked-stereo headphone trim. The property
/// is advertised by the device and identified by CueMix Pro's kHeadphoneTrim
/// model; the record envelope and one-byte attenuation follow the same
/// generation-compatible output-trim protocol. Channel indices are accepted
/// only when freshly discovered from the target itself.
pub(crate) fn set_headphone_trim(
    host: &str,
    target_entity_id: u64,
    output_index: usize,
    trim: OutputTrim,
    timeout: Duration,
) -> Result<(), String> {
    const RETRY_DELAY: Duration = Duration::from_millis(150);
    match set_headphone_trim_once(host, target_entity_id, output_index, trim, timeout) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            thread::sleep(RETRY_DELAY);
            set_headphone_trim_once(host, target_entity_id, output_index, trim, timeout)
                .map_err(|retry_error| {
                    format!(
                        "headphone trim write failed after one fresh-session retry; first attempt: {first_error}; retry: {retry_error}"
                    )
                })
        }
    }
}

fn set_headphone_trim_once(
    host: &str,
    target_entity_id: u64,
    output_index: usize,
    trim: OutputTrim,
    timeout: Duration,
) -> Result<(), String> {
    const CUE_MIX_PROXY_ADDRESS: [u8; 6] = [0x01, 0x00, 0x00, 0x00, 0x01, 0x00];
    const PROPERTY_WRITE_PROTOCOL: [u8; 6] = [0x00, 0x01, 0xf2, 0x00, 0x00, 0x03];
    let mut proxy = AvdeccProxy::connect(host, "/", timeout)?;
    let controller_entity_id = proxy
        .request_entity_id(CUE_MIX_PROXY_ADDRESS, timeout)?
        .entity_id
        .ok_or("AVDECC Proxy did not return a controller identity")?;
    let (next_sequence, state) =
        proxy.start_vendor_state(target_entity_id, controller_entity_id, timeout)?;
    let outputs = headphone_outputs_from_state(&state)?;
    let output = outputs
        .get(output_index)
        .ok_or("that headphone output was not advertised by the device")?;
    let data = output_trim_payload(HEADPHONE_TRIM_PROPERTY, &output.channel_indices, trim);
    proxy.vendor_request(
        target_entity_id,
        controller_entity_id,
        next_sequence,
        PROPERTY_WRITE_PROTOCOL,
        &data,
        timeout,
    )?;
    Ok(())
}

/// Sends one explicitly requested physical line-output trim. Its property and
/// index are freshly discovered from the device before the write is formed.
pub(crate) fn set_line_output_trim(
    host: &str,
    target_entity_id: u64,
    output_index: usize,
    trim: OutputTrim,
    timeout: Duration,
) -> Result<(), String> {
    const RETRY_DELAY: Duration = Duration::from_millis(150);
    match set_line_output_trim_once(host, target_entity_id, output_index, trim, timeout) {
        Ok(()) => Ok(()),
        Err(first_error) => {
            thread::sleep(RETRY_DELAY);
            set_line_output_trim_once(host, target_entity_id, output_index, trim, timeout).map_err(
                |retry_error| {
                    format!(
                        "line-output trim write failed after one fresh-session retry; first attempt: {first_error}; retry: {retry_error}"
                    )
                },
            )
        }
    }
}

fn set_line_output_trim_once(
    host: &str,
    target_entity_id: u64,
    output_index: usize,
    trim: OutputTrim,
    timeout: Duration,
) -> Result<(), String> {
    const CUE_MIX_PROXY_ADDRESS: [u8; 6] = [0x01, 0x00, 0x00, 0x00, 0x01, 0x00];
    const PROPERTY_WRITE_PROTOCOL: [u8; 6] = [0x00, 0x01, 0xf2, 0x00, 0x00, 0x03];
    let mut proxy = AvdeccProxy::connect(host, "/", timeout)?;
    let controller_entity_id = proxy
        .request_entity_id(CUE_MIX_PROXY_ADDRESS, timeout)?
        .entity_id
        .ok_or("AVDECC Proxy did not return a controller identity")?;
    let (next_sequence, state) =
        proxy.start_vendor_state(target_entity_id, controller_entity_id, timeout)?;
    let outputs = line_outputs_from_state(&state)?;
    let output = outputs
        .get(output_index)
        .ok_or("that line output was not advertised by the device")?;
    let data = output_trim_payload(LINE_OUTPUT_TRIM_PROPERTY, &[output.channel_index], trim);
    proxy.vendor_request(
        target_entity_id,
        controller_entity_id,
        next_sequence,
        PROPERTY_WRITE_PROTOCOL,
        &data,
        timeout,
    )?;
    Ok(())
}

fn output_trim_payload(property_id: u16, channel_indices: &[u16], trim: OutputTrim) -> Vec<u8> {
    let attenuation = trim.attenuation();
    let mut data = Vec::with_capacity(channel_indices.len() * 6);
    for channel_index in channel_indices {
        data.extend_from_slice(&property_id.to_be_bytes());
        data.extend_from_slice(&channel_index.to_be_bytes());
        data.push(1);
        data.push(attenuation);
    }
    data
}

fn one_byte_property_payload(property_id: u16, property_index: u16, value: u8) -> [u8; 6] {
    let [property_high, property_low] = property_id.to_be_bytes();
    let [index_high, index_low] = property_index.to_be_bytes();
    [property_high, property_low, index_high, index_low, 1, value]
}

fn line_inputs_from_state(state: &[VendorStateRecord]) -> Result<Vec<LineInput>, String> {
    let gains = one_byte_property_from_state(state, LINE_INPUT_GAIN_PROPERTY, "line-input gain")?;
    let phases =
        one_byte_property_from_state(state, LINE_INPUT_PHASE_PROPERTY, "line-input polarity")?;
    if gains.keys().ne(phases.keys()) {
        return Err("line-input gain and polarity inventories do not match".to_string());
    }
    gains
        .into_iter()
        .map(|(channel_index, gain_db)| {
            if gain_db > LINE_INPUT_GAIN_MAX_DB {
                return Err(
                    "line-input gain state is outside the advertised 0-20 dB range".to_string(),
                );
            }
            let phase = phases[&channel_index];
            let phase_inverted = match phase {
                0 => false,
                1 => true,
                _ => return Err("line-input polarity state is not boolean".to_string()),
            };
            Ok(LineInput {
                channel_index,
                gain_db,
                phase_inverted,
            })
        })
        .collect()
}

fn one_byte_property_from_state(
    state: &[VendorStateRecord],
    property_id: u16,
    description: &str,
) -> Result<BTreeMap<u16, u8>, String> {
    state
        .iter()
        .filter(|record| record.property_id == property_id)
        .map(|record| {
            let [value] = record.value.as_slice() else {
                return Err(format!("{description} state has an unexpected value size"));
            };
            Ok((record.property_index, *value))
        })
        .collect()
}

fn line_outputs_from_state(state: &[VendorStateRecord]) -> Result<Vec<LineOutput>, String> {
    let meter_paths = meter_paths_from_state(state, LINE_OUTPUT_METER_PATH_PROPERTY)?;
    output_channels_from_state(state, LINE_OUTPUT_TRIM_PROPERTY)?
        .into_iter()
        .map(|(channel_index, attenuation)| {
            Ok(LineOutput {
                channel_index,
                attenuation,
                meter_path: meter_paths.get(&channel_index).copied(),
            })
        })
        .collect()
}

fn headphone_outputs_from_state(
    state: &[VendorStateRecord],
) -> Result<Vec<HeadphoneOutput>, String> {
    let channels = output_channels_from_state(state, HEADPHONE_TRIM_PROPERTY)?;
    let meter_paths = meter_paths_from_state(state, HEADPHONE_METER_PATH_PROPERTY)?;
    if !channels.len().is_multiple_of(2) {
        return Err("device advertised an incomplete stereo headphone output".to_string());
    }
    channels
        .chunks_exact(2)
        .enumerate()
        .map(|(output_index, pair)| {
            let [(left_index, left_attenuation), (right_index, right_attenuation)] = pair else {
                unreachable!("chunks_exact always returns stereo pairs");
            };
            if right_index.checked_sub(*left_index) != Some(1) {
                return Err("headphone trim channels are not a consecutive stereo pair".to_string());
            }
            Ok(HeadphoneOutput {
                channel_indices: [*left_index, *right_index],
                attenuation: [*left_attenuation, *right_attenuation],
                meter_path: u16::try_from(output_index)
                    .ok()
                    .and_then(|index| meter_paths.get(&index).copied()),
            })
        })
        .collect()
}

fn meter_paths_from_state(
    state: &[VendorStateRecord],
    property_id: u16,
) -> Result<BTreeMap<u16, MeterPath>, String> {
    state
        .iter()
        .filter(|record| record.property_id == property_id)
        .map(|record| {
            let [property_high, property_low, record_index, channel_index] =
                record.value.as_slice()
            else {
                return Err("meter path state has an unexpected value size".to_string());
            };
            Ok((
                record.property_index,
                MeterPath {
                    property_id: u16::from_be_bytes([*property_high, *property_low]),
                    record_index: *record_index,
                    channel_index: *channel_index,
                },
            ))
        })
        .collect()
}

fn output_channels_from_state(
    state: &[VendorStateRecord],
    property_id: u16,
) -> Result<Vec<(u16, u8)>, String> {
    let channels = state
        .iter()
        .filter(|record| record.property_id == property_id)
        .map(|record| {
            let [attenuation] = record.value.as_slice() else {
                return Err("output trim state has an unexpected value size".to_string());
            };
            if *attenuation > 100 {
                return Err("output trim state has an invalid attenuation".to_string());
            }
            Ok((record.property_index, *attenuation))
        })
        .collect::<Result<BTreeMap<_, _>, String>>()?;
    Ok(channels.into_iter().collect())
}

/// One read-only proxy session owns metering and fresh inventory reads.
/// Background refreshes keep this session and its SSE feed alive.
pub(crate) fn start_mixer_meter_worker(
    host: String,
    target_entity_id: u64,
    timeout: Duration,
) -> MixerMeterWorker {
    let (stop_sender, stop_receiver) = mpsc::channel();
    let (state_sender, state_receiver) = mpsc::channel();
    let meters = Arc::new(MixerMeterFeed::default());
    let worker_meters = Arc::clone(&meters);
    thread::spawn(move || {
        run_mixer_meter_worker(
            &host,
            target_entity_id,
            timeout,
            stop_receiver,
            state_receiver,
            worker_meters,
        )
    });
    MixerMeterWorker {
        stop_sender,
        state_sender,
        meters,
    }
}

fn run_mixer_meter_worker(
    host: &str,
    target_entity_id: u64,
    timeout: Duration,
    stop_receiver: mpsc::Receiver<mpsc::Sender<()>>,
    state_receiver: mpsc::Receiver<VendorSnapshotRequest>,
    meters: Arc<MixerMeterFeed>,
) {
    const RETRY_DELAY: Duration = Duration::from_millis(500);
    const POLL_INTERVAL: Duration = Duration::from_millis(5);
    loop {
        if let Ok(reply) = stop_receiver.try_recv() {
            meters.close();
            let _ = reply.send(());
            return;
        }
        match MixerMeterSession::open(host, target_entity_id, timeout) {
            Ok(mut session) => loop {
                if let Ok(reply) = stop_receiver.try_recv() {
                    drop(session);
                    meters.close();
                    let _ = reply.send(());
                    return;
                }
                if let Ok(request) = state_receiver.try_recv() {
                    if Instant::now() >= request.deadline {
                        let _ = request
                            .reply
                            .send(Err("device refresh expired before it could start".into()));
                    } else {
                        let result = session.read_state(request.deadline, &meters);
                        let error = result.as_ref().err().cloned();
                        let _ = request.reply.send(result.map(VendorSnapshot));
                        if let Some(error) = error {
                            update_mixer_meters(&meters, Vec::new(), Some(error));
                            break;
                        }
                    }
                }
                match session.poll(timeout) {
                    Ok(records) => update_mixer_meters(&meters, records, None),
                    Err(error) => {
                        update_mixer_meters(&meters, Vec::new(), Some(error));
                        break;
                    }
                }
                thread::sleep(POLL_INTERVAL);
            },
            Err(error) => update_mixer_meters(&meters, Vec::new(), Some(error)),
        }
        if let Ok(reply) = stop_receiver.recv_timeout(RETRY_DELAY) {
            meters.close();
            let _ = reply.send(());
            return;
        }
    }
}

fn update_mixer_meters(
    meters: &Arc<MixerMeterFeed>,
    records: Vec<MixerMeterRecord>,
    error: Option<String>,
) {
    meters.publish(records, error);
}

struct MixerMeterSession {
    proxy: AvdeccProxy,
    target_entity_id: u64,
    controller_entity_id: u64,
    next_sequence: u16,
}

impl MixerMeterSession {
    fn open(host: &str, target_entity_id: u64, timeout: Duration) -> Result<Self, String> {
        const CUE_MIX_PROXY_ADDRESS: [u8; 6] = [0x01, 0x00, 0x00, 0x00, 0x01, 0x00];
        let mut proxy = AvdeccProxy::connect(host, "/", timeout)?;
        let controller_entity_id = proxy
            .request_entity_id(CUE_MIX_PROXY_ADDRESS, timeout)?
            .entity_id
            .ok_or("AVDECC Proxy did not return a controller identity")?;
        let (next_sequence, _) =
            proxy.start_vendor_state(target_entity_id, controller_entity_id, timeout)?;
        Ok(Self {
            proxy,
            target_entity_id,
            controller_entity_id,
            next_sequence,
        })
    }

    fn poll(&mut self, timeout: Duration) -> Result<Vec<MixerMeterRecord>, String> {
        const METER_PROTOCOL: [u8; 6] = [0x00, 0x01, 0xf2, 0x00, 0x00, 0x04];
        const CAPTURED_METER_PAGES: usize = 2;
        let sequence = self.next_sequence;
        self.next_sequence = self.next_sequence.wrapping_add(1);
        self.proxy.write_vendor_request(
            self.target_entity_id,
            self.controller_entity_id,
            sequence,
            METER_PROTOCOL,
            &[],
        )?;
        let deadline = Instant::now() + timeout;
        let mut records = Vec::new();
        let mut pages = 0;
        while pages < CAPTURED_METER_PAGES {
            let frame = self
                .proxy
                .read_frame_until(deadline)?
                .ok_or("timed out waiting for CueMix meter response")?;
            let Ok(data) = vendor_response_data(
                &frame,
                self.target_entity_id,
                self.controller_entity_id,
                sequence,
                METER_PROTOCOL,
            ) else {
                continue;
            };
            records.extend(parse_mixer_meter_page(data)?);
            pages += 1;
        }
        Ok(records)
    }

    fn read_state(
        &mut self,
        deadline: Instant,
        meters: &Arc<MixerMeterFeed>,
    ) -> Result<Vec<VendorStateRecord>, String> {
        const PROTOCOL: [u8; 6] = [0, 1, 0xf2, 0, 0, 1];
        // An empty request starts a fresh inventory, even on an existing
        // session. ACK only the preceding STATE sequence; meter requests
        // share the sequence counter but are not part of that ACK chain.
        let mut previous = Vec::new();
        let mut records = Vec::new();
        let mut last_meter = Instant::now();
        for _ in 0..256 {
            let sequence = self.next_sequence;
            self.next_sequence = sequence.wrapping_add(1);
            let response = self.proxy.vendor_request(
                self.target_entity_id,
                self.controller_entity_id,
                sequence,
                PROTOCOL,
                &previous,
                refresh_remaining(deadline)?,
            )?;
            let state = vendor_response_data(
                &response,
                self.target_entity_id,
                self.controller_entity_id,
                sequence,
                PROTOCOL,
            )?;
            if state.is_empty() {
                return Ok(records);
            }
            records.extend(parse_vendor_state_records(state)?);
            previous = sequence.to_be_bytes().to_vec();
            if last_meter.elapsed() >= Duration::from_millis(20) {
                let samples = self.poll(refresh_remaining(deadline)?)?;
                update_mixer_meters(meters, samples, None);
                last_meter = Instant::now();
            }
        }
        Err("CueMix vendor-state snapshot exceeded 256 pages".into())
    }
}

fn refresh_remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|duration| !duration.is_zero())
        .ok_or_else(|| "device refresh timed out".into())
}

fn parse_mixer_meter_page(data: &[u8]) -> Result<Vec<MixerMeterRecord>, String> {
    // Captured ...:04 pages begin with an opaque u16 page counter, followed by
    // property/u8-bank/u8-byte-length records. Each big-endian u16 meter word
    // packs two u8 channels; non-sentinel bytes use 0.5 dB attenuation steps.
    let mut cursor = 2usize;
    let mut records = Vec::new();
    if data.len() < cursor {
        return Err("truncated CueMix meter page counter".to_string());
    }
    while cursor < data.len() {
        let header = data
            .get(cursor..cursor + 4)
            .ok_or("truncated CueMix meter record header")?;
        let property_id = u16::from_be_bytes([header[0], header[1]]);
        let index = header[2];
        let byte_len = header[3] as usize;
        cursor += 4;
        if !byte_len.is_multiple_of(2) {
            return Err("CueMix meter record has an odd sample length".to_string());
        }
        let bytes = data
            .get(cursor..cursor + byte_len)
            .ok_or("truncated CueMix meter record samples")?;
        let values = bytes
            .chunks_exact(2)
            .map(|sample| u16::from_be_bytes([sample[0], sample[1]]))
            .collect();
        cursor += byte_len;
        records.push(MixerMeterRecord {
            property_id,
            index,
            values,
        });
    }
    Ok(records)
}

fn parse_vendor_state_records(data: &[u8]) -> Result<Vec<VendorStateRecord>, String> {
    const HEADER_LEN: usize = 5;
    let mut cursor = 0usize;
    let mut records = Vec::new();
    while cursor < data.len() {
        let header = data
            .get(cursor..cursor + HEADER_LEN)
            .ok_or("truncated CueMix vendor-state record header")?;
        let property_id = u16::from_be_bytes([header[0], header[1]]);
        let property_index = u16::from_be_bytes([header[2], header[3]]);
        let value_len = usize::from(header[4]);
        cursor += HEADER_LEN;
        let value = data
            .get(cursor..cursor + value_len)
            .ok_or("truncated CueMix vendor-state record value")?
            .to_vec();
        cursor += value_len;
        records.push(VendorStateRecord {
            property_id,
            property_index,
            value,
        });
    }
    Ok(records)
}

// IEEE 1722.1-2013 Annex C APPDU: version, type, payload length, EUI-48,
// then a reserved/status u16 before the payload.
const APP_HEADER_LEN: usize = 12;
const APP_MAX_PAYLOAD_LEN: usize = 1490;
const CONNECT_HEADER_LIMIT: usize = 32 * 1024;
const INITIAL_FRAME_WAIT: Duration = Duration::from_millis(250);
const INITIAL_DATA_LIMIT: usize = 8 * 1024;

const APP_NOP: u8 = 0x00;
const APP_ENTITY_ID_REQUEST: u8 = 0x01;
const APP_ENTITY_ID_RESPONSE: u8 = 0x02;
const APP_LINK_UP: u8 = 0x03;
const APP_LINK_DOWN: u8 = 0x04;
const APP_AVDECC_FROM_APS: u8 = 0x05;
const APP_AVDECC_FROM_APC: u8 = 0x06;

#[derive(Clone, Debug, PartialEq, Eq)]
struct AppFrame {
    version: u8,
    message_type: u8,
    address: [u8; 6],
    reserved: u16,
    payload: Vec<u8>,
}

impl AppFrame {
    fn encode(&self) -> Result<Vec<u8>, String> {
        if self.payload.len() > APP_MAX_PAYLOAD_LEN {
            return Err(format!(
                "AVDECC Proxy payload exceeds {APP_MAX_PAYLOAD_LEN} byte limit"
            ));
        }
        let mut bytes = Vec::with_capacity(APP_HEADER_LEN + self.payload.len());
        bytes.push(self.version);
        bytes.push(self.message_type);
        bytes.extend_from_slice(&(self.payload.len() as u16).to_be_bytes());
        bytes.extend_from_slice(&self.address);
        bytes.extend_from_slice(&self.reserved.to_be_bytes());
        bytes.extend_from_slice(&self.payload);
        Ok(bytes)
    }
}

#[derive(Default)]
struct EntityIdResult {
    entity_id: Option<u64>,
    reserved: Option<u16>,
    frames: Vec<AppFrame>,
}

pub(super) struct InitialData {
    pub(super) bytes: Vec<u8>,
    pub(super) chunks: Vec<InitialChunk>,
}

pub(super) struct InitialChunk {
    pub(super) end_offset: usize,
    pub(super) received_after: Duration,
}

struct AvdeccProxy {
    stream: TcpStream,
    buffered: Vec<u8>,
    response: HttpResponse,
}

impl AvdeccProxy {
    fn connect(host: &str, path: &str, timeout: Duration) -> Result<Self, String> {
        let address = parse_proxy_address(host)?;
        validate_proxy_path(path)?;
        let mut stream = connect_with_timeout(&address.socket_address, timeout)?;
        stream
            .set_read_timeout(Some(timeout))
            .map_err(|error| format!("set AVDECC Proxy read timeout failed: {error}"))?;
        stream
            .set_write_timeout(Some(timeout))
            .map_err(|error| format!("set AVDECC Proxy write timeout failed: {error}"))?;
        let request = format!(
            "CONNECT {path} HTTP/1.1\r\n\
             Host: {}\r\n\
             User-Agent: cuemix-848/0.1\r\n\
             Connection: keep-alive\r\n\
             \r\n",
            address.host_header
        );
        stream
            .write_all(request.as_bytes())
            .map_err(|error| format!("write AVDECC Proxy CONNECT failed: {error}"))?;
        let (response, buffered) = read_connect_response(&mut stream, timeout)?;
        if response.status != 200 {
            return Err(format!(
                "AVDECC Proxy CONNECT returned HTTP {} {}",
                response.status, response.reason
            ));
        }
        Ok(Self {
            stream,
            buffered,
            response,
        })
    }

    fn read_available_for(
        &mut self,
        wait: Duration,
        preserve: bool,
    ) -> Result<InitialData, String> {
        let started = Instant::now();
        let mut data = if preserve {
            self.buffered.clone()
        } else {
            std::mem::take(&mut self.buffered)
        };
        let mut chunks = Vec::new();
        if !data.is_empty() {
            chunks.push(InitialChunk {
                end_offset: data.len(),
                received_after: Duration::ZERO,
            });
        }
        while data.len() < INITIAL_DATA_LIMIT {
            let Some(remaining) = wait.checked_sub(started.elapsed()) else {
                break;
            };
            self.stream
                .set_read_timeout(Some(remaining))
                .map_err(|error| format!("set AVDECC Proxy read timeout failed: {error}"))?;
            let mut buffer = [0u8; 1536];
            match self.stream.read(&mut buffer) {
                Ok(0) => return Err("AVDECC Proxy closed the tunnel".to_string()),
                Ok(count) => {
                    let added = append_preview_bytes(
                        &mut data,
                        &mut self.buffered,
                        &buffer[..count],
                        preserve,
                    );
                    if added > 0 {
                        chunks.push(InitialChunk {
                            end_offset: data.len(),
                            received_after: started.elapsed(),
                        });
                    }
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    break
                }
                Err(error)
                    if !data.is_empty()
                        && matches!(
                            error.kind(),
                            std::io::ErrorKind::ConnectionReset
                                | std::io::ErrorKind::ConnectionAborted
                        ) =>
                {
                    break
                }
                Err(error) => return Err(format!("read AVDECC Proxy tunnel failed: {error}")),
            }
        }
        Ok(InitialData {
            bytes: data,
            chunks,
        })
    }

    fn request_entity_id(
        &mut self,
        primary_mac: [u8; 6],
        timeout: Duration,
    ) -> Result<EntityIdResult, String> {
        // This allocates an ephemeral controller identity in the proxy. It is
        // not an AECP command and cannot modify the attached AVDECC entity.
        let request = AppFrame {
            version: 0,
            message_type: APP_ENTITY_ID_REQUEST,
            address: primary_mac,
            reserved: 0,
            payload: vec![0; 8],
        };
        self.stream
            .write_all(&request.encode()?)
            .map_err(|error| format!("write AVDECC Proxy entity ID request failed: {error}"))?;

        let deadline = Instant::now() + timeout;
        let mut frames = Vec::new();
        loop {
            let frame = self
                .read_frame_until(deadline)?
                .ok_or("timed out waiting for AVDECC Proxy entity ID response")?;
            let is_response = is_entity_id_response(&frame, primary_mac);
            frames.push(frame.clone());
            if is_response {
                let entity_id: [u8; 8] = frame
                    .payload
                    .as_slice()
                    .try_into()
                    .map_err(|_| "invalid AVDECC Proxy entity ID response length")?;
                return Ok(EntityIdResult {
                    entity_id: Some(u64::from_be_bytes(entity_id)),
                    reserved: Some(frame.reserved),
                    frames,
                });
            }
        }
    }

    fn start_vendor_state(
        &mut self,
        target_entity_id: u64,
        controller_entity_id: u64,
        timeout: Duration,
    ) -> Result<(u16, Vec<VendorStateRecord>), String> {
        const VENDOR_STATE_PROTOCOL: [u8; 6] = [0x00, 0x01, 0xf2, 0x00, 0x00, 0x01];
        const MAX_INITIAL_STATE_PAGES: usize = 256;

        let mut sequence = 1u16;
        let mut records = Vec::new();
        let mut response = self.vendor_request(
            target_entity_id,
            controller_entity_id,
            sequence,
            VENDOR_STATE_PROTOCOL,
            &[],
            timeout,
        )?;
        for _ in 0..MAX_INITIAL_STATE_PAGES {
            let state = vendor_response_data(
                &response,
                target_entity_id,
                controller_entity_id,
                sequence,
                VENDOR_STATE_PROTOCOL,
            )?;
            if state.is_empty() {
                return Ok((sequence.wrapping_add(1), records));
            }
            records.extend(parse_vendor_state_records(state)?);
            let previous_sequence = sequence;
            sequence = sequence.wrapping_add(1);
            response = self.vendor_request(
                target_entity_id,
                controller_entity_id,
                sequence,
                VENDOR_STATE_PROTOCOL,
                &previous_sequence.to_be_bytes(),
                timeout,
            )?;
        }
        Err("initial CueMix vendor-state snapshot exceeded 256 pages".to_string())
    }

    fn vendor_request(
        &mut self,
        target_entity_id: u64,
        controller_entity_id: u64,
        sequence: u16,
        protocol: [u8; 6],
        data: &[u8],
        timeout: Duration,
    ) -> Result<AppFrame, String> {
        self.write_vendor_request(
            target_entity_id,
            controller_entity_id,
            sequence,
            protocol,
            data,
        )?;
        let deadline = Instant::now() + timeout;
        loop {
            let frame = self
                .read_frame_until(deadline)?
                .ok_or("timed out waiting for CueMix vendor response")?;
            if vendor_response_data(
                &frame,
                target_entity_id,
                controller_entity_id,
                sequence,
                protocol,
            )
            .is_ok()
            {
                return Ok(frame);
            }
        }
    }

    fn write_vendor_request(
        &mut self,
        target_entity_id: u64,
        controller_entity_id: u64,
        sequence: u16,
        protocol: [u8; 6],
        data: &[u8],
    ) -> Result<(), String> {
        const VENDOR_FIXED_LENGTH: usize = 16;
        let control_data_length = VENDOR_FIXED_LENGTH
            .checked_add(data.len())
            .ok_or("vendor command is too large")?;
        let control_data_length =
            u16::try_from(control_data_length).map_err(|_| "vendor command is too large")?;
        let mut payload = Vec::with_capacity(VENDOR_FIXED_LENGTH + 12 + data.len());
        payload.extend_from_slice(&[0xfb, 0x06]);
        payload.extend_from_slice(&control_data_length.to_be_bytes());
        payload.extend_from_slice(&target_entity_id.to_be_bytes());
        payload.extend_from_slice(&controller_entity_id.to_be_bytes());
        payload.extend_from_slice(&sequence.to_be_bytes());
        payload.extend_from_slice(&protocol);
        payload.extend_from_slice(data);
        self.write_frame(&AppFrame {
            version: 0,
            message_type: APP_AVDECC_FROM_APC,
            address: ethernet_address_from_entity_id(target_entity_id),
            reserved: 0,
            payload,
        })
    }

    fn write_frame(&mut self, frame: &AppFrame) -> Result<(), String> {
        self.stream
            .write_all(&frame.encode()?)
            .map_err(|error| format!("write AVDECC Proxy frame failed: {error}"))
    }

    fn read_frame_until(&mut self, deadline: Instant) -> Result<Option<AppFrame>, String> {
        let Some(header) = self.read_exact_until(APP_HEADER_LEN, deadline)? else {
            return Ok(None);
        };
        if header[0] != 0 {
            return Err(format!(
                "unsupported AVDECC Proxy frame version {}",
                header[0]
            ));
        }
        let payload_len = u16::from_be_bytes([header[2], header[3]]) as usize;
        if payload_len > APP_MAX_PAYLOAD_LEN {
            return Err(format!(
                "AVDECC Proxy frame payload exceeds {APP_MAX_PAYLOAD_LEN} byte limit"
            ));
        }
        let payload = self
            .read_exact_until(payload_len, deadline)?
            .ok_or("timed out while reading AVDECC Proxy frame payload")?;
        decode_app_frame(&header, payload).map(Some)
    }

    fn read_exact_until(
        &mut self,
        length: usize,
        deadline: Instant,
    ) -> Result<Option<Vec<u8>>, String> {
        let mut output = Vec::with_capacity(length);
        while output.len() < length {
            if !self.buffered.is_empty() {
                let take = (length - output.len()).min(self.buffered.len());
                output.extend(self.buffered.drain(..take));
                continue;
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return if output.is_empty() {
                    Ok(None)
                } else {
                    Err(timeout_error(&output, length))
                };
            };
            if remaining.is_zero() {
                return if output.is_empty() {
                    Ok(None)
                } else {
                    Err(timeout_error(&output, length))
                };
            }
            self.stream
                .set_read_timeout(Some(remaining))
                .map_err(|error| format!("set AVDECC Proxy read timeout failed: {error}"))?;
            let mut buffer = [0u8; 1536];
            match self.stream.read(&mut buffer) {
                Ok(0) => return Err("AVDECC Proxy closed the tunnel".to_string()),
                Ok(count) => self.buffered.extend_from_slice(&buffer[..count]),
                Err(error)
                    if matches!(
                        error.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) =>
                {
                    return if output.is_empty() {
                        Ok(None)
                    } else {
                        Err(timeout_error(&output, length))
                    };
                }
                Err(error) => return Err(format!("read AVDECC Proxy tunnel failed: {error}")),
            }
        }
        Ok(Some(output))
    }
}

fn append_preview_bytes(
    data: &mut Vec<u8>,
    buffered: &mut Vec<u8>,
    bytes: &[u8],
    preserve: bool,
) -> usize {
    let remaining_capacity = INITIAL_DATA_LIMIT.saturating_sub(data.len());
    data.extend_from_slice(&bytes[..bytes.len().min(remaining_capacity)]);
    if preserve {
        buffered.extend_from_slice(bytes);
    }
    bytes.len().min(remaining_capacity)
}

fn is_entity_id_response(frame: &AppFrame, primary_mac: [u8; 6]) -> bool {
    frame.version == 0
        && frame.message_type == APP_ENTITY_ID_RESPONSE
        && frame.address == primary_mac
}

fn ethernet_address_from_entity_id(entity_id: u64) -> [u8; 6] {
    let entity_id = entity_id.to_be_bytes();
    [
        entity_id[0],
        entity_id[1],
        entity_id[2],
        entity_id[5],
        entity_id[6],
        entity_id[7],
    ]
}

fn vendor_response_data(
    frame: &AppFrame,
    target_entity_id: u64,
    controller_entity_id: u64,
    sequence: u16,
    protocol: [u8; 6],
) -> Result<&[u8], String> {
    const VENDOR_RESPONSE_HEADER_LEN: usize = 28;
    if frame.message_type != APP_AVDECC_FROM_APS
        || frame.address != ethernet_address_from_entity_id(target_entity_id)
        || frame.payload.get(..2) != Some(&[0xfb, 0x07])
        || frame.payload.get(4..12) != Some(target_entity_id.to_be_bytes().as_slice())
        || frame.payload.get(12..20) != Some(controller_entity_id.to_be_bytes().as_slice())
        || frame.payload.get(20..22) != Some(sequence.to_be_bytes().as_slice())
        || frame.payload.get(22..28) != Some(protocol.as_slice())
    {
        return Err("not the expected CueMix vendor response".to_string());
    }
    frame
        .payload
        .get(VENDOR_RESPONSE_HEADER_LEN..)
        .ok_or("truncated CueMix vendor response".to_string())
}

fn timeout_error(received: &[u8], expected: usize) -> String {
    if received.is_empty() {
        "timed out waiting for AVDECC Proxy frame".to_string()
    } else {
        format!(
            "timed out while reading AVDECC Proxy frame: received {} of {expected} bytes ({})",
            received.len(),
            hex_preview(received, 32)
        )
    }
}

fn read_connect_response(
    stream: &mut TcpStream,
    timeout: Duration,
) -> Result<(HttpResponse, Vec<u8>), String> {
    let started = Instant::now();
    let mut bytes = Vec::new();
    loop {
        if let Some(header_end) = find_header_end(&bytes) {
            let response = parse_http_response(&bytes[..header_end])?;
            return Ok((response, bytes[header_end..].to_vec()));
        }
        if bytes.len() >= CONNECT_HEADER_LIMIT {
            return Err("AVDECC Proxy CONNECT response headers are too large".to_string());
        }
        let remaining = timeout
            .checked_sub(started.elapsed())
            .ok_or("timed out waiting for AVDECC Proxy CONNECT response")?;
        stream
            .set_read_timeout(Some(remaining))
            .map_err(|error| format!("set AVDECC Proxy read timeout failed: {error}"))?;
        let mut buffer = [0u8; 1024];
        match stream.read(&mut buffer) {
            Ok(0) => return Err("AVDECC Proxy closed before CONNECT response".to_string()),
            Ok(count) => bytes.extend_from_slice(&buffer[..count]),
            Err(error)
                if matches!(
                    error.kind(),
                    std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                ) =>
            {
                return Err("timed out waiting for AVDECC Proxy CONNECT response".to_string())
            }
            Err(error) => {
                return Err(format!(
                    "read AVDECC Proxy CONNECT response failed: {error}"
                ))
            }
        }
    }
}

fn find_header_end(bytes: &[u8]) -> Option<usize> {
    bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .map(|index| index + 4)
}

fn decode_app_frame(header: &[u8], payload: Vec<u8>) -> Result<AppFrame, String> {
    if header.len() != APP_HEADER_LEN {
        return Err("invalid AVDECC Proxy frame header length".to_string());
    }
    if !is_known_v0_message_type(header[1]) {
        return Err("invalid AVDECC Proxy frame message type".to_string());
    }
    let payload_len = u16::from_be_bytes([header[2], header[3]]) as usize;
    if payload.len() != payload_len {
        return Err("invalid AVDECC Proxy frame payload length".to_string());
    }
    let address: [u8; 6] = header[4..10]
        .try_into()
        .map_err(|_| "invalid AVDECC Proxy frame address")?;
    let reserved = u16::from_be_bytes([header[10], header[11]]);
    Ok(AppFrame {
        version: header[0],
        message_type: header[1],
        address,
        reserved,
        payload,
    })
}

fn is_known_v0_message_type(message_type: u8) -> bool {
    matches!(
        message_type,
        APP_NOP
            | APP_ENTITY_ID_REQUEST
            | APP_ENTITY_ID_RESPONSE
            | APP_LINK_UP
            | APP_LINK_DOWN
            | APP_AVDECC_FROM_APS
            | APP_AVDECC_FROM_APC
            | 0xff
    )
}

fn decode_complete_v0_frames(bytes: &[u8]) -> Vec<AppFrame> {
    let mut offset = 0;
    let mut frames = Vec::new();
    while bytes.get(offset) == Some(&0) && bytes.len() - offset >= APP_HEADER_LEN {
        let header = &bytes[offset..offset + APP_HEADER_LEN];
        let payload_len = u16::from_be_bytes([header[2], header[3]]) as usize;
        let frame_end = offset + APP_HEADER_LEN + payload_len;
        if frame_end > bytes.len() {
            break;
        }
        let payload = bytes[offset + APP_HEADER_LEN..frame_end].to_vec();
        let Ok(frame) = decode_app_frame(header, payload) else {
            break;
        };
        frames.push(frame);
        offset = frame_end;
    }
    frames
}

#[cfg(test)]
#[path = "avdecc_tests.rs"]
mod tests;
pub(crate) use avdecc_probe::ProbeTiming;
