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
    fn reads_are_left_to_cors() {
        assert!(allowed(&Method::GET, &h(Some("cross-site"))));
        assert!(allowed(&Method::OPTIONS, &h(Some("cross-site"))));
    }
}
