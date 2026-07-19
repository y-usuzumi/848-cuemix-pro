use std::fs::File;
use std::io::{self, Write};
use std::path::PathBuf;

use crate::device::{json_escape, DeviceClient};

#[derive(Clone)]
pub(crate) struct ProbeResult {
    path: String,
    status: Option<u16>,
    reason: String,
    bytes: usize,
    preview: String,
    error: Option<String>,
}

pub(crate) fn probe_device(client: &DeviceClient) -> Vec<ProbeResult> {
    probe_paths()
        .iter()
        .map(|path| {
            eprintln!("probe {path}");
            match client.request("GET", path, None) {
                Ok(response) => ProbeResult {
                    path: path.to_string(),
                    status: Some(response.status),
                    reason: response.reason,
                    bytes: response.body.len(),
                    preview: compact_preview(&response.body),
                    error: None,
                },
                Err(error) => ProbeResult {
                    path: path.to_string(),
                    status: None,
                    reason: String::new(),
                    bytes: 0,
                    preview: String::new(),
                    error: Some(error),
                },
            }
        })
        .collect()
}

fn probe_paths() -> &'static [&'static str] {
    &[
        "/",
        "/apiversion",
        "/uid",
        "/api",
        "/api/",
        "/api/rest",
        "/api/rest/",
        "/api/rest/device",
        "/api/rest/devices",
        "/api/rest/system",
        "/api/rest/audio",
        "/api/rest/audio/mixers",
        "/api/rest/audio/mixers/0",
        "/api/rest/audio/mixers/0/faders",
        "/api/rest/audio/mixers/0/faders/0",
        "/api/rest/control",
        "/api/rest/control/mixers",
        "/api/rest/control/mixers/0",
        "/api/rest/routing",
        "/api/rest/patchbay",
        "/api/rest/clock",
        "/api/rest/avb",
        "/datastore",
        "/datastore/",
        "/datastore/ext",
        "/datastore/ext/caps",
        "/datastore/ext/caps/mixer",
        "/datastore/ext/caps/router",
        "/datastore/ext/caps/avb",
        "/datastore/ext/obank",
        "/datastore/ext/ibank",
        "/datastore/mix",
        "/datastore/mix/chan",
        "/datastore/mix/chan/1",
        "/datastore/mix/chan/1/matrix",
        "/datastore/mix/chan/1/matrix/fader",
        "/datastore/mon",
        "/datastore/avb",
        "/datastore/cfg",
        "/datastore/dev",
    ]
}

pub(crate) fn write_probe_results(
    results: &[ProbeResult],
    save: Option<PathBuf>,
) -> Result<(), String> {
    let mut stdout = io::stdout();
    for result in results {
        writeln!(stdout, "{}", probe_result_json(result)).map_err(|err| err.to_string())?;
    }
    if let Some(path) = save {
        let mut file = File::create(&path).map_err(|err| format!("create {:?}: {err}", path))?;
        for result in results {
            writeln!(file, "{}", probe_result_json(result)).map_err(|err| err.to_string())?;
        }
        eprintln!("saved {}", path.display());
    }
    Ok(())
}

pub(crate) fn probe_result_json(result: &ProbeResult) -> String {
    let status = result
        .status
        .map(|value| value.to_string())
        .unwrap_or_else(|| "null".to_string());
    let error = result
        .error
        .as_ref()
        .map(|value| format!("\"{}\"", json_escape(value)))
        .unwrap_or_else(|| "null".to_string());
    format!(
        "{{\"path\":\"{}\",\"status\":{},\"reason\":\"{}\",\"bytes\":{},\"preview\":\"{}\",\"error\":{}}}",
        json_escape(&result.path),
        status,
        json_escape(&result.reason),
        result.bytes,
        json_escape(&result.preview),
        error
    )
}

fn compact_preview(body: &str) -> String {
    let compact = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if compact.chars().count() > 300 {
        format!("{}...", compact.chars().take(300).collect::<String>())
    } else {
        compact
    }
}
