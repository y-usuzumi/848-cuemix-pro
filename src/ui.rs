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
