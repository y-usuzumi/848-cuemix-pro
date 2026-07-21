use crate::discovery::{browser_control_hosts, DiscoveryResult};

pub(crate) fn render(default_host: &str, session_token: &str) -> String {
    include_str!("ui.html")
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

pub(crate) fn render_discovery(results: &[DiscoveryResult]) -> String {
    let devices = results
        .iter()
        .map(|result| {
            let links = browser_control_hosts(result)
                .into_iter()
                .map(|host| {
                    format!(
                        r#"<a class="open" href="/?host={}">Open {}</a>"#,
                        query_component(&host),
                        html_escape(&host)
                    )
                })
                .collect::<Vec<_>>()
                .join("");
            let addresses = result
                .addresses
                .iter()
                .map(|address| format!(r#"<code>{}</code>"#, html_escape(address)))
                .collect::<Vec<_>>()
                .join(" ");
            let txt = result
                .txt
                .iter()
                .map(|value| format!(r#"<li>{}</li>"#, html_escape(value)))
                .collect::<Vec<_>>()
                .join("");
            format!(
                r#"<article class="device"><h2>{}</h2><p class="host">{} · AVDECC Proxy {}</p><p class="addresses">{}</p><div class="opens">{}</div><ul>{}</ul></article>"#,
                html_escape(&result.instance),
                html_escape(&result.host),
                result.port,
                addresses,
                if links.is_empty() {
                    r#"<span class="muted">No usable HTTP control address was advertised.</span>"#
                } else {
                    &links
                },
                txt
            )
        })
        .collect::<Vec<_>>()
        .join("");
    let devices = if devices.is_empty() {
        r#"<p class="empty">No 848 was found. Stop and restart this server to scan again.</p>"#
            .to_string()
    } else {
        devices
    };
    format!(
        r#"<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width, initial-scale=1"><title>cuemix-848 discovery</title><style>
:root{{color-scheme:light dark;--bg:#f6f7f4;--ink:#171916;--muted:#5a6157;--panel:#fff;--line:#cfd8c8;--accent:#1f7a5f}}@media(prefers-color-scheme:dark){{:root{{--bg:#121512;--ink:#f4f6f1;--muted:#a8b0a4;--panel:#1c211d;--line:#394238;--accent:#60c1a1}}}}*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--ink);font:14px/1.45 system-ui,-apple-system,BlinkMacSystemFont,Segoe UI,sans-serif}}main{{width:min(900px,calc(100vw - 32px));margin:0 auto;padding:28px 0}}h1{{margin:0;font-size:24px}}.sub,.muted,.host{{color:var(--muted)}}.device{{margin-top:16px;padding:18px;border:1px solid var(--line);border-radius:8px;background:var(--panel)}}h2{{margin:0;font-size:17px}}p{{margin:8px 0}}code{{display:inline-block;margin:2px 6px 2px 0;padding:2px 5px;border-radius:4px;background:color-mix(in srgb,var(--panel),var(--ink) 8%);font-family:ui-monospace,SFMono-Regular,Consolas,monospace}}.opens{{display:flex;flex-wrap:wrap;gap:8px;margin:12px 0}}.open{{padding:7px 10px;border-radius:5px;background:var(--accent);color:white;text-decoration:none;font-weight:650}}ul{{margin:10px 0 0;padding-left:20px;color:var(--muted);font-size:12px}}.empty{{margin-top:22px;color:var(--muted)}}</style></head><body><main><h1>cuemix-848</h1><p class="sub">Discovered AVDECC devices</p>{}</main></body></html>"#,
        devices
    )
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
    fn renders_discovered_device_links_only_for_usable_control_hosts() {
        let result = DiscoveryResult {
            instance: "848._avdecc._tcp.local".to_string(),
            host: "848.local".to_string(),
            port: 17221,
            addresses: vec!["192.168.4.166".to_string(), "fe80::1".to_string()],
            txt: vec!["Version=1".to_string()],
        };
        let html = render_discovery(&[result]);
        assert!(html.contains("/?host=192.168.4.166"));
        assert!(!html.contains("/?host=%5Bfe80%3A%3A1%5D"));
        assert!(html.contains("Version=1"));
    }
}
