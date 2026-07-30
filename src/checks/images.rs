//! Docker image update check.
//!
//! For every image behind a running container, compares the local image
//! digest against the digest the registry currently serves for that tag.
//! A mismatch means a newer image has been published.
//!
//! Registry queries are throttled through a persistent cache (images.json)
//! so frequent sweeps (e.g. a 5-minute cron) do not hammer registries:
//! each image is re-queried only after `check_interval` (default 6h).
//!
//! Skipped entirely: images pinned by digest (`@sha256:` in the reference,
//! a deliberate choice) and locally built images (no RepoDigests).

use std::collections::HashMap;
use std::io::{Read, Write as _};
use std::net::{TcpStream, ToSocketAddrs};
use std::process::Command;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{ClientConfig, ClientConnection, StreamOwned};

use super::{CheckResult, CheckStatus, Section, tls_client_config};
use crate::config::{ImagesConfig, parse_duration_secs};
use crate::state::{ImageCache, ImageCacheEntry};

const DEFAULT_INTERVAL_SECS: i64 = 6 * 3600;

const MANIFEST_ACCEPT: &str = "application/vnd.docker.distribution.manifest.list.v2+json, \
     application/vnd.oci.image.index.v1+json, \
     application/vnd.docker.distribution.manifest.v2+json, \
     application/vnd.oci.image.manifest.v1+json";

pub(crate) fn check_images(config: &Option<ImagesConfig>) -> Vec<CheckResult> {
    let (interval, ignore) = match config {
        Some(c) => {
            if !c.enabled {
                return vec![];
            }
            let interval = parse_duration_secs(&c.check_interval).unwrap_or(DEFAULT_INTERVAL_SECS);
            (interval, c.ignore.clone())
        }
        None => (DEFAULT_INTERVAL_SECS, vec![]),
    };

    // No docker (or no daemon) means nothing to check; stay quiet like
    // other checks with empty config.
    let refs = match running_images() {
        Ok(r) => r,
        Err(_) => return vec![],
    };
    if refs.is_empty() {
        return vec![];
    }

    let mut pinned = 0usize;
    let mut local = 0usize;
    let mut ignored = 0usize;
    let mut unreachable = 0usize;
    let mut current = 0usize;
    let mut outdated: Vec<ImageRef> = Vec::new();

    let mut cache = ImageCache::load();
    let now = chrono::Utc::now().timestamp();
    let tls_config = tls_client_config();

    for raw in &refs {
        if ignore.iter().any(|p| raw.contains(p.as_str())) {
            ignored += 1;
            continue;
        }
        if raw.contains("@sha256:") {
            pinned += 1;
            continue;
        }

        let local_digests = match repo_digests(raw) {
            Ok(d) => d,
            Err(_) => {
                unreachable += 1;
                continue;
            }
        };
        if local_digests.is_empty() {
            local += 1;
            continue;
        }

        let image = parse_image_ref(raw);

        let cached = cache.images.get(raw).cloned();
        let fresh = cached
            .as_ref()
            .is_some_and(|e| now - e.checked_at < interval);

        let remote_digest = if fresh {
            cached.and_then(|e| e.remote_digest)
        } else {
            let fetched = fetch_remote_digest(&tls_config, &image).ok();
            // On failure keep the last known digest but still bump the
            // timestamp, so a broken registry is retried once per interval
            // instead of on every sweep.
            let entry = ImageCacheEntry {
                remote_digest: fetched.clone().or(cached.and_then(|e| e.remote_digest)),
                checked_at: now,
            };
            let digest = entry.remote_digest.clone();
            cache.images.insert(raw.clone(), entry);
            digest
        };

        match remote_digest {
            Some(remote) => {
                let up_to_date = local_digests
                    .iter()
                    .any(|d| d.ends_with(&format!("@{}", remote)));
                if up_to_date {
                    current += 1;
                } else {
                    outdated.push(image);
                }
            }
            None => unreachable += 1,
        }
    }

    // Drop cache entries for images no longer running.
    cache.images.retain(|k, _| refs.contains(k));
    let _ = cache.save();

    let mut parts = vec![format!("{} current", current)];
    if pinned > 0 {
        parts.push(format!("{} pinned", pinned));
    }
    if local > 0 {
        parts.push(format!("{} local", local));
    }
    if ignored > 0 {
        parts.push(format!("{} ignored", ignored));
    }
    if unreachable > 0 {
        parts.push(format!("{} unreachable", unreachable));
    }

    let mut results = vec![CheckResult {
        section: Section::Updates,
        name: "images".into(),
        status: CheckStatus::Ok,
        summary: parts.join(", "),
        ..Default::default()
    }];

    outdated.sort_by(|a, b| a.short.cmp(&b.short));
    for image in outdated {
        results.push(CheckResult {
            section: Section::Updates,
            name: image.short.clone(),
            status: CheckStatus::Warning,
            summary: format!("image update available ({})", image.raw),
            ..Default::default()
        });
    }

    results
}

// --- Docker queries ---

fn running_images() -> Result<Vec<String>, String> {
    let output = Command::new("docker")
        .args(["ps", "-q"])
        .output()
        .map_err(|e| format!("docker: {}", e))?;

    if !output.status.success() {
        return Err("docker ps failed".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let ids: Vec<&str> = stdout.lines().map(str::trim).filter(|l| !l.is_empty()).collect();
    if ids.is_empty() {
        return Ok(vec![]);
    }

    // docker ps strips a `@sha256:` pin from the displayed image, so read
    // the reference the container was actually created with instead.
    let output = Command::new("docker")
        .args(["inspect", "--format", "{{.Config.Image}}"])
        .args(&ids)
        .output()
        .map_err(|e| format!("docker: {}", e))?;

    if !output.status.success() {
        return Err("docker inspect failed".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let mut refs: Vec<String> = stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect();
    refs.sort();
    refs.dedup();
    Ok(refs)
}

fn repo_digests(image: &str) -> Result<Vec<String>, String> {
    let output = Command::new("docker")
        .args([
            "image",
            "inspect",
            "--format",
            "{{join .RepoDigests \"\\n\"}}",
            image,
        ])
        .output()
        .map_err(|e| format!("docker: {}", e))?;

    if !output.status.success() {
        return Err("docker image inspect failed".into());
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Ok(stdout
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty())
        .map(String::from)
        .collect())
}

// --- Image reference parsing ---

#[derive(Debug, PartialEq)]
struct ImageRef {
    /// Registry API host to query (docker.io maps to registry-1.docker.io).
    host: String,
    /// Repository path, e.g. "library/redis" or "immich-app/immich-server".
    repo: String,
    tag: String,
    /// The reference as reported by docker ps.
    raw: String,
    /// Short display name (last repository segment).
    short: String,
}

fn parse_image_ref(raw: &str) -> ImageRef {
    let (host, rest) = match raw.split_once('/') {
        Some((first, rest))
            if first.contains('.') || first.contains(':') || first == "localhost" =>
        {
            (first.to_string(), rest.to_string())
        }
        _ => ("docker.io".to_string(), raw.to_string()),
    };

    let (repo_part, tag) = match rest.rsplit_once(':') {
        Some((r, t)) if !t.contains('/') => (r.to_string(), t.to_string()),
        _ => (rest.clone(), "latest".to_string()),
    };

    let repo = if host == "docker.io" && !repo_part.contains('/') {
        format!("library/{}", repo_part)
    } else {
        repo_part
    };

    let host = if host == "docker.io" {
        "registry-1.docker.io".to_string()
    } else {
        host
    };

    let short = repo.rsplit('/').next().unwrap_or(&repo).to_string();

    ImageRef {
        host,
        repo,
        tag,
        raw: raw.to_string(),
        short,
    }
}

// --- Registry protocol ---

fn fetch_remote_digest(tls_config: &Arc<ClientConfig>, image: &ImageRef) -> Result<String, String> {
    let path = format!("/v2/{}/manifests/{}", image.repo, image.tag);
    let accept = ("Accept".to_string(), MANIFEST_ACCEPT.to_string());

    let resp = https_request(tls_config, &image.host, "HEAD", &path, std::slice::from_ref(&accept))?;

    let resp = if resp.status == 401 {
        let challenge = resp
            .headers
            .get("www-authenticate")
            .ok_or("401 without WWW-Authenticate")?;
        let (realm, service) =
            parse_bearer_challenge(challenge).ok_or("unsupported auth challenge")?;
        let token = fetch_token(tls_config, &realm, &service, &image.repo)?;
        let auth = ("Authorization".to_string(), format!("Bearer {}", token));
        https_request(tls_config, &image.host, "HEAD", &path, &[accept, auth])?
    } else {
        resp
    };

    if resp.status != 200 {
        return Err(format!("manifest HEAD returned {}", resp.status));
    }

    resp.headers
        .get("docker-content-digest")
        .cloned()
        .ok_or_else(|| "no Docker-Content-Digest header".into())
}

/// Parse `Bearer realm="...",service="..."` into (realm, service).
fn parse_bearer_challenge(header: &str) -> Option<(String, String)> {
    let rest = header.trim().strip_prefix("Bearer ")?;
    let mut realm = None;
    let mut service = None;
    for part in rest.split(',') {
        let (key, value) = part.trim().split_once('=')?;
        let value = value.trim_matches('"');
        match key.trim() {
            "realm" => realm = Some(value.to_string()),
            "service" => service = Some(value.to_string()),
            _ => {}
        }
    }
    Some((realm?, service?))
}

fn fetch_token(
    tls_config: &Arc<ClientConfig>,
    realm: &str,
    service: &str,
    repo: &str,
) -> Result<String, String> {
    let rest = realm
        .strip_prefix("https://")
        .ok_or("token realm is not https")?;
    let (host, base_path) = match rest.find('/') {
        Some(i) => (&rest[..i], &rest[i..]),
        None => (rest, "/"),
    };
    let path = format!(
        "{}?service={}&scope=repository:{}:pull",
        base_path, service, repo
    );

    let resp = https_request(tls_config, host, "GET", &path, &[])?;
    if resp.status != 200 {
        return Err(format!("token request returned {}", resp.status));
    }

    let json: serde_json::Value =
        serde_json::from_slice(&resp.body).map_err(|e| format!("token response: {}", e))?;
    json.get("token")
        .or_else(|| json.get("access_token"))
        .and_then(|v| v.as_str())
        .map(String::from)
        .ok_or_else(|| "no token in response".into())
}

// --- Minimal HTTPS client (headers + body) ---

struct HttpResponse {
    status: u16,
    /// Header names lowercased.
    headers: HashMap<String, String>,
    body: Vec<u8>,
}

fn https_request(
    tls_config: &Arc<ClientConfig>,
    host: &str,
    method: &str,
    path: &str,
    extra_headers: &[(String, String)],
) -> Result<HttpResponse, String> {
    let timeout = Duration::from_secs(10);

    let socket_addr = format!("{}:443", host)
        .to_socket_addrs()
        .map_err(|e| format!("resolve failed: {}", e))?
        .next()
        .ok_or_else(|| format!("no addresses for {}", host))?;

    let tcp = TcpStream::connect_timeout(&socket_addr, timeout)
        .map_err(|e| format!("connect failed: {}", e))?;
    tcp.set_read_timeout(Some(timeout))
        .map_err(|e| format!("set timeout: {}", e))?;
    tcp.set_write_timeout(Some(timeout))
        .map_err(|e| format!("set timeout: {}", e))?;

    let mut request = format!(
        "{} {} HTTP/1.1\r\nHost: {}\r\nUser-Agent: alertpaca\r\nConnection: close\r\n",
        method, path, host
    );
    for (key, value) in extra_headers {
        request.push_str(&format!("{}: {}\r\n", key, value));
    }
    request.push_str("\r\n");

    let server_name = ServerName::try_from(host.to_string())
        .map_err(|e| format!("invalid hostname: {}", e))?;
    let conn = ClientConnection::new(tls_config.clone(), server_name)
        .map_err(|e| format!("TLS error: {}", e))?;
    let mut tls = StreamOwned::new(conn, tcp);

    tls.write_all(request.as_bytes())
        .map_err(|e| format!("write failed: {}", e))?;

    let mut raw = Vec::new();
    let mut buf = [0u8; 4096];
    loop {
        match tls.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => raw.extend_from_slice(&buf[..n]),
            // Servers that skip close_notify surface as an error at EOF;
            // whatever was read so far is the full response.
            Err(_) if !raw.is_empty() => break,
            Err(e) => return Err(format!("read failed: {}", e)),
        }
    }

    parse_http_response(&raw)
}

fn parse_http_response(raw: &[u8]) -> Result<HttpResponse, String> {
    let sep = raw
        .windows(4)
        .position(|w| w == b"\r\n\r\n")
        .ok_or("malformed response (no header terminator)")?;

    let head = std::str::from_utf8(&raw[..sep]).map_err(|_| "invalid response headers")?;
    let mut lines = head.split("\r\n");

    let status_line = lines.next().ok_or("empty response")?;
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or("invalid status line")?;

    let mut headers = HashMap::new();
    for line in lines {
        if let Some((key, value)) = line.split_once(':') {
            headers.insert(key.trim().to_lowercase(), value.trim().to_string());
        }
    }

    let mut body = raw[sep + 4..].to_vec();
    if headers
        .get("transfer-encoding")
        .is_some_and(|v| v.to_lowercase().contains("chunked"))
    {
        body = dechunk(&body);
    }

    Ok(HttpResponse {
        status,
        headers,
        body,
    })
}

/// Decode a chunked transfer-encoded body. Tolerant: stops at the first
/// malformed chunk and returns what was decoded so far.
fn dechunk(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::new();
    let mut pos = 0;

    while pos < data.len() {
        let line_end = match data[pos..].windows(2).position(|w| w == b"\r\n") {
            Some(i) => pos + i,
            None => break,
        };
        let size_str = match std::str::from_utf8(&data[pos..line_end]) {
            Ok(s) => s.split(';').next().unwrap_or("").trim(),
            Err(_) => break,
        };
        let size = match usize::from_str_radix(size_str, 16) {
            Ok(s) => s,
            Err(_) => break,
        };
        if size == 0 {
            break;
        }
        let chunk_start = line_end + 2;
        let chunk_end = chunk_start + size;
        if chunk_end > data.len() {
            out.extend_from_slice(&data[chunk_start..]);
            break;
        }
        out.extend_from_slice(&data[chunk_start..chunk_end]);
        pos = chunk_end + 2; // skip trailing \r\n
    }

    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_ref_dockerhub_official() {
        let r = parse_image_ref("redis:8");
        assert_eq!(r.host, "registry-1.docker.io");
        assert_eq!(r.repo, "library/redis");
        assert_eq!(r.tag, "8");
        assert_eq!(r.short, "redis");
    }

    #[test]
    fn test_parse_ref_dockerhub_user() {
        let r = parse_image_ref("pihole/pihole:latest");
        assert_eq!(r.host, "registry-1.docker.io");
        assert_eq!(r.repo, "pihole/pihole");
        assert_eq!(r.tag, "latest");
    }

    #[test]
    fn test_parse_ref_dockerhub_explicit() {
        let r = parse_image_ref("docker.io/library/redis:8");
        assert_eq!(r.host, "registry-1.docker.io");
        assert_eq!(r.repo, "library/redis");
    }

    #[test]
    fn test_parse_ref_ghcr() {
        let r = parse_image_ref("ghcr.io/home-assistant/home-assistant:stable");
        assert_eq!(r.host, "ghcr.io");
        assert_eq!(r.repo, "home-assistant/home-assistant");
        assert_eq!(r.tag, "stable");
        assert_eq!(r.short, "home-assistant");
    }

    #[test]
    fn test_parse_ref_no_tag() {
        let r = parse_image_ref("alpine/socat");
        assert_eq!(r.repo, "alpine/socat");
        assert_eq!(r.tag, "latest");
    }

    #[test]
    fn test_parse_bearer_challenge() {
        let (realm, service) = parse_bearer_challenge(
            "Bearer realm=\"https://auth.docker.io/token\",service=\"registry.docker.io\"",
        )
        .unwrap();
        assert_eq!(realm, "https://auth.docker.io/token");
        assert_eq!(service, "registry.docker.io");
    }

    #[test]
    fn test_parse_bearer_challenge_with_scope() {
        let (realm, service) = parse_bearer_challenge(
            "Bearer realm=\"https://ghcr.io/token\",service=\"ghcr.io\",scope=\"repository:x/y:pull\"",
        )
        .unwrap();
        assert_eq!(realm, "https://ghcr.io/token");
        assert_eq!(service, "ghcr.io");
    }

    #[test]
    fn test_parse_http_response() {
        let raw = b"HTTP/1.1 200 OK\r\nDocker-Content-Digest: sha256:abc\r\nContent-Length: 2\r\n\r\nok";
        let resp = parse_http_response(raw).unwrap();
        assert_eq!(resp.status, 200);
        assert_eq!(resp.headers.get("docker-content-digest").unwrap(), "sha256:abc");
        assert_eq!(resp.body, b"ok");
    }

    #[test]
    fn test_dechunk() {
        let body = b"4\r\nWiki\r\n5\r\npedia\r\n0\r\n\r\n";
        assert_eq!(dechunk(body), b"Wikipedia");
    }
}
