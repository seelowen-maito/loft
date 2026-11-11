//! Refuses state-changing requests that a browser says come from another site.
//!
//! CORS only decides whether a page may *read* a response. A "simple" cross-site request (a POST with no
//! body or a form body) is still sent and handled, so without this any page open in the user's browser
//! could, for example, `POST /api/restart`. Browsers mark every request with `Sec-Fetch-Site`; our own
//! frontend is `same-origin` (directly, through the Vite proxy or `tailscale serve`) and a typed URL or
//! bookmark is `none`. Clients that aren't browsers (the iOS app, curl, agents' MCP calls) don't send it.

use axum::extract::Request;
use axum::http::{HeaderMap, Method, StatusCode};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};

pub fn allowed(method: &Method, headers: &HeaderMap) -> bool {
    if matches!(*method, Method::GET | Method::HEAD | Method::OPTIONS) {
        return true;
    }
    match headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        None => true,
        Some(site) => matches!(site, "same-origin" | "none"),
    }
}

/// For reads that return private data (device tokens, conversation text): CORS is permissive, so
/// without this any website open in the user's browser could read them. Non-browser clients send no
/// `Sec-Fetch-Site` and pass.
pub fn same_site_read(headers: &HeaderMap) -> Result<(), crate::api::ApiError> {
    match headers.get("sec-fetch-site").and_then(|v| v.to_str().ok()) {
        None | Some("same-origin" | "none") => Ok(()),
        Some(_) => Err(crate::api::ApiError(StatusCode::FORBIDDEN, "cross-site request refused".into())),
    }
}

/// Whether `Host` names this Mac: a web page on another site can rebind its domain to 127.0.0.1 and then
/// reach the API from the browser, but the browser still sends the page's own name as `Host`. `value` is the
/// header as sent, with or without a port.
fn host_ok(value: &str) -> bool {
    match host_known(value) {
        Ok(known) => known,
        Err(host) => tailnet_name().is_some_and(|t| t.eq_ignore_ascii_case(&host)),
    }
}

/// `Ok` when the name is decided without asking Tailscale; `Err` with the name when only its name could allow it.
fn host_known(value: &str) -> Result<bool, String> {
    let value = value.trim().to_ascii_lowercase();
    let host = match value.strip_prefix('[') {
        Some(rest) => rest.split(']').next().unwrap_or_default(),
        None => value.split(':').next().unwrap_or_default(),
    }
    .trim_end_matches('.');
    if matches!(host, "localhost" | "127.0.0.1" | "::1") {
        return Ok(true);
    }
    // Any IP literal: a rebound domain still sends its name as `Host`, never an address.
    if host.parse::<std::net::IpAddr>().is_ok() {
        return Ok(true);
    }
    let mac = mac_name();
    if !mac.is_empty() && (host == mac || host == format!("{}.local", mac.trim_end_matches(".local"))) {
        return Ok(true);
    }
    if in_list(&std::env::var("LOFT_ALLOWED_HOSTS").unwrap_or_default(), host) {
        return Ok(true);
    }
    Err(host.to_string())
}

/// Whether `host` is one of the comma-separated names in `list` (names only, no ports).
fn in_list(list: &str, host: &str) -> bool {
    !host.is_empty() && list.split(',').any(|h| h.trim().eq_ignore_ascii_case(host))
}

/// This Mac's name, as `gethostname` gives it.
fn mac_name() -> String {
    let mut buf = [0u8; 256];
    let ok = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) } == 0;
    let n = buf.iter().position(|&b| b == 0).unwrap_or(0);
    if ok { String::from_utf8_lossy(&buf[..n]).to_ascii_lowercase() } else { String::new() }
}

/// The Tailscale name. Asking the CLI is a process, so a failed look is not repeated for 30 seconds.
fn tailnet_name() -> Option<String> {
    static MISS: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
    let mut miss = MISS.lock().unwrap();
    if miss.is_some_and(|t| t.elapsed().as_secs() < 30) {
        return None;
    }
    let h = crate::tailnet::host();
    *miss = h.is_none().then(std::time::Instant::now);
    h
}

/// Refuses a request whose `Host` isn't this Mac's. A request with no `Host` (not a browser) passes.
/// A browser that still holds the `__Host-loft` cookie of the removed pairing is told to drop it.
pub async fn local_host(req: Request, next: Next) -> Response {
    let old_cookie = req.headers().get_all(axum::http::header::COOKIE).iter().any(|c| c.to_str().is_ok_and(|c| c.contains("__Host-loft=")));
    let ok: Result<(), String> = match req.headers().get(axum::http::header::HOST).map(|v| v.to_str().unwrap_or_default().to_string()) {
        None => Ok(()),
        // Asking Tailscale for its name is a process: off the async threads.
        Some(v) => match host_known(&v) {
            Ok(known) => known.then_some(()).ok_or(v),
            Err(_) => {
                let h = v.clone();
                if tokio::task::spawn_blocking(move || host_ok(&h)).await.unwrap_or(false) { Ok(()) } else { Err(v) }
            }
        },
    };
    if let Err(host) = ok {
        return (StatusCode::FORBIDDEN, format!("host \"{host}\" refused: not this Mac's name; add it to LOFT_ALLOWED_HOSTS (comma-separated, no port)")).into_response();
    }
    let mut res = next.run(req).await;
    if old_cookie {
        res.headers_mut().append(axum::http::header::SET_COOKIE, "__Host-loft=; Max-Age=0; Path=/; Secure; HttpOnly; SameSite=Strict".parse().unwrap());
    }
    res
}

pub async fn same_origin_writes(req: Request, next: Next) -> Response {
    if allowed(req.method(), req.headers()) {
        next.run(req).await
    } else {
        (StatusCode::FORBIDDEN, "cross-site request refused").into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn h(site: Option<&str>) -> HeaderMap {
        let mut h = HeaderMap::new();
        if let Some(s) = site {
            h.insert("sec-fetch-site", s.parse().unwrap());
        }
        h
    }

    #[test]
    fn writes_only_from_our_own_pages_or_non_browsers() {
        assert!(allowed(&Method::POST, &h(None)), "iOS app / curl / MCP");
        assert!(allowed(&Method::POST, &h(Some("same-origin"))));
        assert!(allowed(&Method::POST, &h(Some("none"))));
        for site in ["cross-site", "same-site"] {
            for m in [Method::POST, Method::PUT, Method::PATCH, Method::DELETE] {
                assert!(!allowed(&m, &h(Some(site))), "{m} from {site}");
            }
        }
    }

    #[test]
    fn only_this_macs_names_are_hosts() {
        for ok in ["localhost", "localhost:7420", "127.0.0.1:7420", "[::1]:7420", "100.101.102.103", "100.64.0.1:7420", "192.168.1.20:7420", "10.0.0.1", "100.128.0.1", "[fe80::1]:7420", "[fd7a::1]", "[fd7a:115c:a1e0::1]", "[fd7a:115c:a1e0:ab12::1]:7420", "LOCALHOST."] {
            assert!(host_ok(ok), "{ok}");
        }
        for bad in ["evil.example", "evil.example:7420", "127.0.0.1.evil.example", "localhost.evil.example", "", "evil.example:7420@localhost"] {
            assert!(!host_ok(bad), "{bad}");
        }
        assert!(host_ok(&format!("{}:7420", mac_name())));
        assert!(host_ok(&format!("{}.local", mac_name().trim_end_matches(".local"))));
    }

    #[test]
    fn extra_hosts_come_from_the_environment() {
        let list = "other.example, Extra.Example";
        assert!(in_list(list, "extra.example") && in_list(list, "other.example"));
        assert!(!in_list(list, "evil.example") && !in_list(list, "") && !in_list("a,,b", ""));
    }

    #[test]
    fn reads_are_left_to_cors() {
        assert!(allowed(&Method::GET, &h(Some("cross-site"))));
        assert!(allowed(&Method::OPTIONS, &h(Some("cross-site"))));
    }
}
