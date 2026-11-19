use std::thread;
use std::time::Duration;

use base64::Engine;
use url::Url;

const ENDPOINT: &str = "https://api.tinify.com/shrink";
/// ponytail: cap the response at 1GB so a bad payload can't exhaust memory.
const MAX_BODY: u64 = 1024 * 1024 * 1024;
const USER_AGENT: &str = concat!("Bamboo/", env!("CARGO_PKG_VERSION"));

/// Compress `image` with the Tinify API and return the compressed bytes.
///
/// Matches the official client: one retry after 500ms on connection failure
/// or HTTP 5xx, and Basic auth with username `api`.
pub fn shrink(key: &str, image: &[u8]) -> Result<Vec<u8>, String> {
    let key = key.trim();
    if key.is_empty() {
        return Err("Provide an API key.".into());
    }
    if !looks_like_image(image) {
        return Err("File is not a PNG or JPEG.".into());
    }

    let auth = basic_auth(key);
    let agent = agent();
    let posted = request(&agent, "POST", ENDPOINT, Some(image), &auth)?;
    let location = posted
        .location
        .or_else(|| output_url(&posted.body).ok())
        .ok_or_else(|| "Tinify response did not include a download URL".to_string())?;
    let download = resolve_location(&location)?;
    trusted_download_url(&download)?;
    let fetched = request(&agent, "GET", download.as_str(), None, &auth)?;
    if !looks_like_image(&fetched.body) {
        return Err("Tinify did not return a PNG or JPEG.".into());
    }
    Ok(fetched.body)
}

fn looks_like_image(bytes: &[u8]) -> bool {
    bytes.starts_with(b"\x89PNG\r\n\x1a\n") || bytes.starts_with(b"\xFF\xD8\xFF")
}

fn basic_auth(key: &str) -> String {
    let token = base64::engine::general_purpose::STANDARD.encode(format!("api:{key}"));
    format!("Basic {token}")
}

fn output_url(body: &[u8]) -> Result<String, String> {
    let value: serde_json::Value = serde_json::from_slice(body)
        .map_err(|_| "Tinify response did not include a download URL".to_string())?;
    value
        .get("output")
        .and_then(|output| output.get("url"))
        .and_then(|url| url.as_str())
        .filter(|url| !url.is_empty())
        .map(str::to_owned)
        .ok_or_else(|| "Tinify response did not include a download URL".to_string())
}

fn resolve_location(location: &str) -> Result<Url, String> {
    let base = Url::parse("https://api.tinify.com/").expect("endpoint is a valid URL");
    base.join(location)
        .map_err(|_| "Tinify returned an invalid download URL".to_string())
}

fn trusted_download_url(url: &Url) -> Result<(), String> {
    if url.scheme() != "https" {
        return Err("Tinify returned a non-HTTPS download URL".into());
    }
    let host = url.host_str().unwrap_or("");
    if host == "tinify.com" || host.ends_with(".tinify.com") {
        Ok(())
    } else {
        Err("Tinify returned an unexpected download URL".into())
    }
}

/// Same text the Tinify Node client puts on `Error.message`.
fn format_api_error(status: u16, body: &[u8]) -> String {
    match serde_json::from_slice::<serde_json::Value>(body) {
        Ok(value) => {
            let message = value
                .get("message")
                .and_then(|message| message.as_str())
                .filter(|message| !message.is_empty())
                .unwrap_or("No message was provided");
            match value.get("error").and_then(|error| error.as_str()) {
                Some(kind) if !kind.is_empty() => format!("{message} (HTTP {status}/{kind})"),
                _ => format!("{message} (HTTP {status})"),
            }
        }
        Err(err) => format!("Error while parsing response: {err}"),
    }
}

struct RawResponse {
    location: Option<String>,
    body: Vec<u8>,
}

fn agent() -> ureq::Agent {
    // ponytail: 120s cap so a hung upload doesn't spin forever.
    let config = ureq::Agent::config_builder()
        .timeout_global(Some(Duration::from_secs(120)))
        .http_status_as_error(false)
        .https_only(true)
        .build();
    ureq::Agent::new_with_config(config)
}

fn request(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    auth: &str,
) -> Result<RawResponse, String> {
    let mut last = String::from("Tinify request failed");
    for attempt in 0..2 {
        match once(agent, method, url, body, auth) {
            Ok(response) if (200..300).contains(&response.status) => {
                return Ok(RawResponse {
                    location: response.location,
                    body: response.body,
                });
            }
            Ok(response) if response.status >= 500 && attempt == 0 => {
                last = format_api_error(response.status, &response.body);
                thread::sleep(Duration::from_millis(500));
            }
            Ok(response) => return Err(format_api_error(response.status, &response.body)),
            Err(err) if attempt == 0 => {
                last = err;
                thread::sleep(Duration::from_millis(500));
            }
            Err(err) => return Err(err),
        }
    }
    Err(last)
}

struct Attempt {
    status: u16,
    location: Option<String>,
    body: Vec<u8>,
}

fn once(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    auth: &str,
) -> Result<Attempt, String> {
    let result = if method == "POST" {
        agent
            .post(url)
            .header("Authorization", auth)
            .header("User-Agent", USER_AGENT)
            .send(body.unwrap_or(&[]))
    } else {
        agent
            .get(url)
            .header("Authorization", auth)
            .header("User-Agent", USER_AGENT)
            .call()
    };
    match result {
        Ok(mut response) => {
            let status = response.status().as_u16();
            let location = response
                .headers()
                .get("location")
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let body = response
                .body_mut()
                .with_config()
                .limit(MAX_BODY)
                .read_to_vec()
                .map_err(|err| format!("Error while connecting: {err}"))?;
            Ok(Attempt {
                status,
                location,
                body,
            })
        }
        Err(err) => Err(format!("Error while connecting: {err}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn auth_header_is_basic_api_key() {
        assert_eq!(basic_auth("abc"), "Basic YXBpOmFiYw==");
    }

    #[test]
    fn empty_key_and_non_images_do_not_hit_the_network() {
        assert_eq!(
            shrink("  ", b"\x89PNG\r\n\x1a\n").unwrap_err(),
            "Provide an API key."
        );
        assert_eq!(
            shrink("key", b"hello").unwrap_err(),
            "File is not a PNG or JPEG."
        );
        assert!(looks_like_image(b"\x89PNG\r\n\x1a\nrest"));
        assert!(looks_like_image(b"\xFF\xD8\xFFrest"));
        assert!(!looks_like_image(b"\x89PNG"));
    }

    #[test]
    fn api_error_matches_the_tinify_client() {
        let body = br#"{"error":"Unauthorized","message":"Credentials are invalid"}"#;
        assert_eq!(
            format_api_error(401, body),
            "Credentials are invalid (HTTP 401/Unauthorized)"
        );
        assert_eq!(
            format_api_error(400, br#"{"error":"Bad"}"#),
            "No message was provided (HTTP 400/Bad)"
        );
        assert!(format_api_error(500, b"nope").starts_with("Error while parsing response:"));
    }

    #[test]
    fn output_url_comes_from_the_shrink_body() {
        let body = br#"{"output":{"size":1,"url":"https://api.tinify.com/output/abc"}}"#;
        assert_eq!(
            output_url(body).unwrap(),
            "https://api.tinify.com/output/abc"
        );
        assert!(output_url(br#"{"output":{}}"#).is_err());
    }

    #[test]
    fn download_urls_must_be_https_tinify_hosts() {
        let ok = [
            "https://api.tinify.com/output/abc",
            "https://cdn.tinify.com/x",
            "https://user:pass@api.tinify.com/output/abc",
        ];
        for url in ok {
            let parsed = resolve_location(url).unwrap();
            assert!(trusted_download_url(&parsed).is_ok(), "{url}");
        }
        let relative = resolve_location("/output/abc").unwrap();
        assert_eq!(relative.host_str(), Some("api.tinify.com"));
        assert!(trusted_download_url(&relative).is_ok());

        let bad = [
            "http://api.tinify.com/output/abc",
            "https://evil.example/output",
            "https://api.tinify.com.evil.example/output",
            "https://nottinify.com/output",
            "https://api.tinify.com@evil.example/output",
            "https://user:pass@evil.example/output",
        ];
        for url in bad {
            let parsed = resolve_location(url).unwrap();
            assert!(trusted_download_url(&parsed).is_err(), "{url}");
        }
    }
}
