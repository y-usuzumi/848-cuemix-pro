use crate::discovery::{browser_control_hosts, DiscoveryResult};

pub(crate) fn render(default_host: &str, session_token: &str, home: bool) -> String {
    include_str!("ui.html")
        .replace("__CONSOLE_CSS__", include_str!("console.css"))
        .replace("__CONSOLE_SHELL_CSS__", include_str!("console_shell.css"))
        .replace("__LAYOUT_JS__", include_str!("ui_layout.js"))
        .replace("__CONSOLE_PANELS__", include_str!("console_panels.html"))
        .replace("__CONSOLE_MODEL__", include_str!("console_model.js"))
        .replace("__MONITOR_JS__", include_str!("monitor.js"))
        .replace("__SLIDER_QUEUE_JS__", include_str!("slider_queue.js"))
        .replace("__DB_ENTRY_JS__", include_str!("db_entry.js"))
        .replace("__CONSOLE_JS__", include_str!("console.js"))
        .replace(
            "__DEVICE_HOME_LINK__",
            if home {
                r#"<a class="device-home" href="/">← Devices</a>"#
            } else {
                ""
            },
        )
        .replace("__DEFAULT_HOST__", &html_escape(default_host))
        .replace("__SESSION_TOKEN__", session_token)
}

fn html_escape(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

pub(crate) fn render_discovery(
    results: &[DiscoveryResult],
    session_token: &str,
    error: Option<&str>,
) -> String {
    let error = error
        .map(|error| format!("Discovery unavailable: {error}. You can still connect by IP."))
        .unwrap_or_default();
    include_str!("home.html")
        .replace("__HOME_JS__", include_str!("home.js"))
        .replace("__SESSION_TOKEN__", session_token)
        .replace("__DISCOVERY_ERROR__", &html_escape(&error))
        .replace("__DEVICES__", &render_device_list(results))
}

pub(crate) fn render_device_list(results: &[DiscoveryResult]) -> String {
    if results.is_empty() {
        return r#"<div class="empty"><strong>No devices found yet</strong><p>Check that your 848 is powered on and connected to the network, then scan again or enter its IP address.</p></div>"#.into();
    }
    results.iter().map(|result| {
        let name = result.instance.strip_suffix("._avdecc._tcp.local").unwrap_or(&result.instance);
        let links = browser_control_hosts(result).into_iter().map(|host| format!(
            r#"<a class="device-address" href="/?host={}" data-device-host="{}"><code>{}</code><span>Connect ↗</span></a>"#,
            query_component(&host), html_escape(&host), html_escape(&host)
        )).collect::<Vec<_>>().join("");
        format!(
            r#"<article class="device"><div class="device-head"><span class="device-icon" aria-hidden="true">≋</span><div><h3>{}</h3><p class="device-host">{}</p></div></div><div class="device-addresses">{}</div></article>"#,
            html_escape(name), html_escape(&result.host),
            if links.is_empty() { r#"<p>No usable address was advertised. Connect by IP instead.</p>"# } else { &links },
        )
    }).collect::<Vec<_>>().join("")
}

fn query_component(input: &str) -> String {
    input
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_page_escapes_device_text_and_keeps_manual_connection_on_errors() {
        let result = DiscoveryResult {
            instance: "<script>alert(1)</script>._avdecc._tcp.local".into(),
            host: "\"<img src=x>".into(),
            port: 17221,
            addresses: vec!["[bad]".into(), "fe80::1%eth2".into()],
            txt: Vec::new(),
        };
        let page = render_discovery(&[result], "secret", Some("<network failed>"));
        assert!(!page.contains("<script>alert(1)</script>"));
        assert!(!page.contains("<img src=x>"));
        assert!(page.contains("&lt;network failed&gt;"));
        assert!(page.contains("/?host=%5Bfe80%3A%3A1%25eth2%5D"));
        assert!(page.contains("id=\"manualConnect\""));
        assert!(page.contains("Scan again"));
        assert!(!page.contains("__HOME_JS__"));
        assert!(!render("192.168.1.50", "secret", false).contains("← Devices"));
        assert!(render("192.168.1.50", "secret", true).contains("← Devices"));
    }

    #[test]
    fn renders_named_workspaces_and_persistent_monitor_controls() {
        let html = render("192.168.4.166", "session-token", false);
        assert!(!html.contains("__DB_ENTRY_JS__"));
        assert!(html.contains("CueMixDb.mount(document"));

        for (tab, panel) in [
            ("inputs", "panel-inputs"),
            ("outputs", "panel-outputs"),
            ("patchbay", "panel-patchbay"),
            ("routing", "panel-routing"),
            ("mixer", "panel-mixer"),
            ("aux", "panel-aux"),
            ("diagnostics", "panel-diagnostics"),
        ] {
            assert!(html.contains(&format!("id=\"tab-{tab}\"")));
            assert!(html.contains(&format!("aria-controls=\"{panel}\"")));
            assert!(html.contains(&format!("id=\"{panel}\"")));
            assert!(html.contains(&format!("aria-labelledby=\"tab-{tab}\"")));
        }

        let inputs = html
            .split("id=\"panel-inputs\"")
            .nth(1)
            .expect("inputs panel")
            .split("</section>")
            .next()
            .expect("inputs panel contents");
        assert!(inputs.contains("Mic Preamps"));
        assert!(inputs.contains("Line Inputs"));
        assert!(inputs.contains("id=\"lineInputOut\""));
        assert!(!inputs.contains("Analog Output Gains"));

        let outputs = html
            .split("id=\"panel-outputs\"")
            .nth(1)
            .expect("outputs panel")
            .split("</section>")
            .next()
            .expect("outputs panel contents");
        assert!(outputs.contains("Line Output Gains"));
        assert!(outputs.contains("Phones"));
        assert!(!outputs.contains("id=\"phoneOut\""));
        assert!(html.contains("class=\"monitor-rail console-panel\""));
        assert!(html.contains("id=\"phoneOut\""));
        assert!(html.contains("id=\"monitorSetupControls\""));
        assert!(html.contains("id=\"mixPages\""));
        assert!(html.contains("id=\"auxPages\""));
        assert!(!html.contains("__LAYOUT_JS__"));
        assert!(!html.contains("__CONSOLE_SHELL_CSS__"));
        assert!(!outputs.contains("Mic Preamps"));
        assert!(html.contains("fetchJson('/api/outputs?'"));
        assert!(html.contains("/api/outputs/line-trim"));
        assert!(html.contains("fetchJson('/api/inputs/lines?'"));
        assert!(html.contains("const lineInputPath = '/datastore/ext/ibank/1'"));
        assert!(html.contains("const lineOutputPath = '/datastore/ext/obank/0'"));
        assert!(html.contains("/api/inputs/line-phase"));
        assert!(html.contains("switchMarkup(index, 'phase', 'Ø', data[key('phase')], 'Polarity')"));
        assert!(html
            .contains("id=\"line-in-${index}-phase\" type=\"checkbox\" aria-label=\"Polarity\""));
        assert!(html.contains("class=\"channel-name-button\""));
        assert!(html.contains("id=\"theme\" aria-label=\"Color theme\""));
        assert!(html.contains("const themeStorageKey = 'cuemix-848-theme'"));
        assert!(html.contains("const savedTheme = localStorage.getItem('cuemix-848-theme')"));
        assert!(html.contains("--accent-ink: #07140f"));
        assert!(html.contains("color: var(--accent-ink)"));
        assert!(html.contains("background: var(--panel); color: var(--ink)"));
        assert!(html.contains("function bindPreampNameEditor("));
        assert!(html.contains("function bindChannelNameEditor("));
        assert!(html.contains("id=\"line-in-${index}-name\""));
        assert!(html.contains("id=\"out-${index}-name\""));
        assert!(html.contains("path: lineInputFieldPath(index, 'name')"));
        assert!(html.contains("path: lineOutputFieldPath(index, output, 'name')"));
        assert!(html.contains("class=\"gain-fader\" type=\"range\" orient=\"vertical\""));
        assert!(html.contains(".switches { display: flex; justify-content: center"));
        assert!(!html.contains("class=\"switch-state\""));
        assert!(html.contains("channelDisplayLabel(data[`ch/${channel}/name`]"));
        assert!(html.contains("meterLaneMarkup(`mic-${index}-meter`)"));
        assert!(html.contains("meterLaneMarkup(`line-in-${index}-meter`)"));
        assert!(html.contains("meterLaneMarkup(`out-${index}-meter`)"));
        assert!(html.contains("meterLaneMarkup(`phone-${index}-meter-l`)"));
        assert!(html.contains("meterLaneMarkup(`phone-${index}-meter-r`)"));
        assert!(html.contains("new EventSource('/api/mixer/meters/events?'"));
        assert!(html.contains("requestAnimationFrame"));
        assert!(html.contains("!Array.isArray(meters.records) || !meters.records.length"));
        for mark in [
            "'−∞'", "'−48'", "'−36'", "'−24'", "'−12'", "'−6'", "'−3'", "'clip'",
        ] {
            assert!(html.contains(mark), "missing meter mark {mark}");
        }
        assert!(html.contains("function meterScaleMarkup()"));
        assert!(html.contains("const meterDbPerStep = 0.5"));
        assert!(html.contains("return -Number(value) * meterDbPerStep"));
        assert!(html.contains("const percent = meterPercent(value)"));
        assert!(html.contains("fill.style.clipPath = `inset(${100 - percent}% 0 0)`"));
        assert!(html.contains("class=\"vertical-meter-column\""));
        assert!(html.contains("const meterPeakHoldMs = 1000"));
        assert!(html.contains("function renderMeterPeak("));
        assert!(html.contains("class=\"meter-peak\""));
        assert!(html.contains("style=\"bottom:${point.percent}%\""));
        assert!(!html.contains("setInterval(loadMeters"));
        assert!(!html.contains("const outputPath = '/datastore/ext/obank/0'"));
    }

    #[test]
    fn renders_discovered_device_links_only_for_usable_control_hosts() {
        let result = DiscoveryResult {
            instance: "848._avdecc._tcp.local".to_string(),
            host: "848.local".to_string(),
            port: 17221,
            addresses: vec!["192.168.4.166".to_string(), "fe80::1".to_string()],
            txt: vec!["Version=1".to_string()],
        };
        let html = render_discovery(&[result], "session-token", None);
        assert!(html.contains("/?host=192.168.4.166"));
        assert!(!html.contains("/?host=%5Bfe80%3A%3A1%5D"));
        assert!(html.contains("Connect by IP"));
        assert!(html.contains("data-device-host=\"192.168.4.166\""));
        assert!(!html.contains("__DEVICES__"));
    }
}
