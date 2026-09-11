//! CueMix routing and mixer records. See docs/console-protocol.md for the
//! installed-client and read-only device evidence behind these mappings.
use super::{AvdeccProxy, VendorStateRecord};
use crate::device::json_escape;
use std::collections::{BTreeMap, BTreeSet};
use std::time::Duration;

const Q24: f64 = 16_777_216.0;
const PROTOCOL: [u8; 6] = [0, 1, 0xf2, 0, 0, 3];
const MAX_CHANGES: usize = 32;

#[derive(Clone, Debug)]
pub(crate) struct ConsoleState {
    records: BTreeMap<(u16, u16), Vec<u8>>,
}

#[derive(Debug)]
pub(crate) struct ConsoleChange {
    operation: String,
    target: String,
    index: u16,
    expected: Vec<u8>,
    value: String,
}

#[derive(Debug, PartialEq)]
struct WriteRecord {
    property: u16,
    index: u16,
    value: Vec<u8>,
}

#[derive(Debug)]
pub(crate) struct ConsoleWriteError {
    pub(crate) applied: usize,
    pub(crate) conflict: bool,
    pub(crate) message: String,
}

fn relevant(property: u16) -> bool {
    matches!(property,
        0x8020..=0x8025 | 0x8027..=0x802d | 0x802f |
        0x03e8 | 0x03e9 | 0x83f8 | 0x83f9 | 0x03fa | 0x03fb |
        0x0403 | 0x0404 | 0x0411 | 0x0412 |
        0x841a | 0x842b | 0x0420 | 0x0421 | 0x0429 |
        0x842e | 0x843f | 0x0434 | 0x0435 | 0x043c | 0x043d |
        0x0448 | 0x0449 | 0x93ac..=0x93b1 | 0x1b5b)
}

impl ConsoleState {
    pub(super) fn from_records(records: Vec<VendorStateRecord>) -> Result<Self, String> {
        let mut result = BTreeMap::new();
        for record in records.into_iter().filter(|r| relevant(r.property_id)) {
            if result
                .insert((record.property_id, record.property_index), record.value)
                .is_some()
            {
                return Err("device supplied duplicate console records".into());
            }
        }
        if result.is_empty() {
            return Err("device did not advertise a console".into());
        }
        Ok(Self { records: result })
    }

    pub(crate) fn json(&self) -> String {
        let records = self
            .records
            .iter()
            .map(|(&(property, index), value)| {
                format!("[{}, {}, \"{}\"]", property, index, hex(value))
            })
            .collect::<Vec<_>>()
            .join(",");
        format!(r#"{{"records":[{records}]}}"#)
    }

    fn get(&self, property: u16, index: u16) -> Result<&[u8], String> {
        self.records
            .get(&(property, index))
            .map(Vec::as_slice)
            .ok_or_else(|| {
                format!("control {property:04x}:{index:04x} is not advertised by this device")
            })
    }

    fn indices(&self, property: u16) -> impl Iterator<Item = u16> + '_ {
        self.records
            .keys()
            .filter(move |(p, _)| *p == property)
            .map(|(_, i)| *i)
    }

    fn sources(&self) -> BTreeSet<Vec<u8>> {
        let mut sources = BTreeSet::from([vec![0, 0, 0, 0]]);
        for (names, path) in [(0x8025, 0x138c), (0x8020, 0x13ac)] {
            for index in self.indices(names).filter(|i| *i < 256) {
                let [hi, lo] = u16::to_be_bytes(path);
                sources.insert(vec![hi, lo, 0, index as u8]);
            }
        }
        // Host names use bank/channel indices, but host paths use a linear channel.
        for index in self.indices(0x8023).filter(|i| i >> 8 < 16 && i & 255 < 8) {
            sources.insert(vec![
                0x13,
                0xb0,
                0,
                ((index >> 8) * 8 + (index & 255)) as u8,
            ]);
        }
        for index in self.indices(0x8022).filter(|i| i >> 8 < 2 && i & 255 < 8) {
            sources.insert(vec![0x13, 0xae, (index >> 8) as u8, index as u8]);
        }
        for index in self.indices(0x8024).filter(|i| *i < 128) {
            let stream = index / 8;
            // The last AVB listener may be a media-clock stream, not audio.
            if self
                .get(0x1b5b, stream)
                .is_ok_and(|format| format.len() == 6 && format[4] > (index % 8) as u8)
            {
                sources.insert(vec![0x13, 0xaf, stream as u8, (index % 8) as u8]);
            }
        }
        for (property, bank) in [
            (0x03e8, 0),
            (0x0420, 1),
            (0x0448, 2),
            (0x0403, 3),
            (0x0434, 4),
        ] {
            for index in self.indices(property).filter(|i| *i < 256) {
                sources.insert(vec![0x13, 0xad, bank, index as u8]);
            }
        }
        sources
    }

    fn resolve(&self, change: &ConsoleChange) -> Result<WriteRecord, String> {
        let (property, index, value) = match change.operation.as_str() {
            "route" => {
                let property = match change.target.as_str() {
                    "line" => 0x93ac,
                    "mixer" => 0x93ad,
                    "optical" => 0x93ae,
                    "network" => 0x93af,
                    "host" => 0x93b0,
                    "phones" => 0x93b1,
                    _ => return Err("unknown routing destination group".into()),
                };
                let path = unhex(&change.value)?;
                if !self.sources().contains(&path) {
                    return Err("source is not advertised by this device".into());
                }
                (property, change.index, path)
            }
            "level" | "pan" => {
                if change.index > 255 {
                    return Err("mixer input index exceeds one byte".into());
                }
                self.get(0x03e8, change.index)?;
                let (fader, pan, bus) = self.bus(&change.target)?;
                let property = if change.operation == "level" {
                    fader
                } else {
                    pan
                };
                let value = if change.operation == "level" {
                    encode_level(&change.value)?
                } else {
                    encode_pan(&change.value)?
                };
                (
                    property,
                    (change.index << 8) | bus,
                    value.to_be_bytes().to_vec(),
                )
            }
            "mute" | "solo" => {
                if change.target != "input" {
                    return Err("mute and solo require an input channel".into());
                }
                self.get(0x03e8, change.index)?;
                (
                    if change.operation == "mute" {
                        0x03fb
                    } else {
                        0x03fa
                    },
                    change.index,
                    vec![parse_bool(&change.value)?],
                )
            }
            "master" | "master-mute" | "pre" => {
                let (_, _, bus) = self.bus(&change.target)?;
                if (change.target.starts_with("aux-") && change.index != bus)
                    || (!change.target.starts_with("aux-") && change.index > 1)
                {
                    return Err("bus index does not match the selected mix".into());
                }
                let bus = change.index;
                let (gain, mute, pre) = if change.target == "main" {
                    (0x0420, 0x0421, None)
                } else if change.target == "reverb" {
                    (0x0434, 0x0435, Some(0x043c))
                } else {
                    (0x0403, 0x0404, Some(0x0411))
                };
                match change.operation.as_str() {
                    "master" => (
                        gain,
                        bus,
                        encode_level(&change.value)?.to_be_bytes().to_vec(),
                    ),
                    "master-mute" => (mute, bus, vec![parse_bool(&change.value)?]),
                    _ => (
                        pre.ok_or("Main does not have a pre/post send switch")?,
                        bus,
                        vec![parse_bool(&change.value)?],
                    ),
                }
            }
            _ => return Err("unknown console operation".into()),
        };
        let current = self.get(property, index)?;
        if current.len() != value.len() {
            return Err("device control has an unexpected value size".into());
        }
        if current != change.expected {
            return Err("conflict: the device value changed; refresh and review this edit".into());
        }
        Ok(WriteRecord {
            property,
            index,
            value,
        })
    }

    fn bus(&self, target: &str) -> Result<(u16, u16, u16), String> {
        match target {
            "main" => {
                self.get(0x0420, 0)?;
                Ok((0x841a, 0x842b, 0))
            }
            "reverb" => {
                self.get(0x0434, 0)?;
                Ok((0x842e, 0x843f, 0))
            }
            _ => {
                let index = target
                    .strip_prefix("aux-")
                    .and_then(|s| s.parse::<u16>().ok())
                    .filter(|i| *i < 256)
                    .ok_or("unknown mixer bus")?;
                self.get(0x0403, index)?;
                Ok((0x83f8, 0x83f9, index))
            }
        }
    }

    fn prepare(&self, changes: &[ConsoleChange]) -> Result<Vec<WriteRecord>, String> {
        let mut keys = BTreeSet::new();
        let mut result = Vec::new();
        for change in changes {
            let record = self.resolve(change)?;
            if !keys.insert((record.property, record.index)) {
                return Err("duplicate control in edit batch".into());
            }
            if self.get(record.property, record.index)? != record.value {
                result.push(record);
            }
        }
        Ok(result)
    }
}

pub(crate) fn parse_changes(input: &str) -> Result<Vec<ConsoleChange>, String> {
    if input.is_empty() || input.len() > 8192 {
        return Err("expected 1–32 console edits".into());
    }
    let mut changes = Vec::new();
    for item in input.split(';') {
        let parts = item.split(':').collect::<Vec<_>>();
        if parts.len() != 5 || changes.len() == MAX_CHANGES {
            return Err("expected at most 32 operation:target:index:previous:value edits".into());
        }
        let index = parts[2]
            .parse::<u16>()
            .map_err(|_| "invalid console channel index")?;
        let expected = unhex(parts[3])?;
        if !matches!(expected.len(), 1 | 4) {
            return Err("invalid previous control value".into());
        }
        changes.push(ConsoleChange {
            operation: parts[0].into(),
            target: parts[1].into(),
            index,
            expected,
            value: parts[4].into(),
        });
    }
    Ok(changes)
}

fn encode_level(value: &str) -> Result<u32, String> {
    if value == "-inf" {
        return Ok(0);
    }
    let db = value
        .parse::<f64>()
        .map_err(|_| "level must be -inf or -90 through +12 dB")?;
    if !db.is_finite() || !(-90.0..=12.0).contains(&db) {
        return Err("level must be -inf or -90 through +12 dB".into());
    }
    // CueMix IFaderImpl and OFaderImpl multiply linear gain by 2^24 and truncate.
    Ok((10_f64.powf(db / 20.0) * Q24) as u32)
}

fn encode_pan(value: &str) -> Result<u32, String> {
    let pan = value
        .parse::<f64>()
        .map_err(|_| "pan must be between -1 and 1")?;
    if !pan.is_finite() || !(-1.0..=1.0).contains(&pan) {
        return Err("pan must be between -1 and 1".into());
    }
    Ok(((pan + 1.0) * 0.5 * Q24) as u32)
}

fn parse_bool(value: &str) -> Result<u8, String> {
    match value {
        "0" => Ok(0),
        "1" => Ok(1),
        _ => Err("toggle must be 0 or 1".into()),
    }
}

fn unhex(value: &str) -> Result<Vec<u8>, String> {
    if value.is_empty()
        || value.len() > 8
        || !value.len().is_multiple_of(2)
        || !value.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("invalid encoded control value".into());
    }
    (0..value.len())
        .step_by(2)
        .map(|i| {
            u8::from_str_radix(&value[i..i + 2], 16)
                .map_err(|_| "invalid encoded control value".into())
        })
        .collect()
}

fn hex(value: &[u8]) -> String {
    value.iter().map(|b| format!("{b:02x}")).collect()
}

fn open(
    host: &str,
    target: u64,
    timeout: Duration,
) -> Result<(AvdeccProxy, u64, u16, ConsoleState), String> {
    let mut proxy = AvdeccProxy::connect(host, "/", timeout)?;
    let controller = proxy
        .request_entity_id([1, 0, 0, 0, 1, 0], timeout)?
        .entity_id
        .ok_or("no controller identity")?;
    let (sequence, records) = proxy.start_vendor_state(target, controller, timeout)?;
    Ok((
        proxy,
        controller,
        sequence,
        ConsoleState::from_records(records)?,
    ))
}

pub(crate) fn write_console(
    host: &str,
    target: u64,
    changes: &[ConsoleChange],
    timeout: Duration,
) -> Result<usize, ConsoleWriteError> {
    let error = |message: String| ConsoleWriteError {
        applied: 0,
        conflict: message.starts_with("conflict:"),
        message,
    };
    let (mut proxy, controller, mut sequence, state) =
        open(host, target, timeout).map_err(error)?;
    // Validate the ENTIRE batch against one fresh snapshot before the first setter.
    let writes = state.prepare(changes).map_err(error)?;
    for (applied, record) in writes.iter().enumerate() {
        let mut payload = Vec::with_capacity(9);
        payload.extend(record.property.to_be_bytes());
        payload.extend(record.index.to_be_bytes());
        payload.push(record.value.len() as u8);
        payload.extend(&record.value);
        let response = proxy
            .vendor_request(target, controller, sequence, PROTOCOL, &payload, timeout)
            .and_then(|frame| validate_ack(&frame.payload));
        if let Err(message) = response {
            return Err(ConsoleWriteError { applied, conflict: false,
                message: format!("{message}; {applied} changes acknowledged. The last write outcome is unknown; refresh before retrying.") });
        }
        sequence = sequence.wrapping_add(1);
    }
    Ok(writes.len())
}

fn validate_ack(payload: &[u8]) -> Result<(), String> {
    if payload.len() < 28 {
        return Err("truncated console acknowledgement".into());
    }
    let status = payload[2] >> 3;
    let length = (u16::from_be_bytes([payload[2], payload[3]]) & 0x7ff) as usize;
    if status != 0 {
        return Err(format!(
            "device rejected console write with status {status}"
        ));
    }
    if length != payload.len() - 12 {
        return Err("invalid console acknowledgement length".into());
    }
    Ok(())
}

impl ConsoleWriteError {
    pub(crate) fn json(&self) -> String {
        format!(
            r#"{{"error":"{}","applied":{},"conflict":{}}}"#,
            json_escape(&self.message),
            self.applied,
            self.conflict
        )
    }
}

#[cfg(test)]
#[path = "avdecc_console_tests.rs"]
mod tests;
