//! Primary source capture, SSRF defensive guards, and snapshot storage.
//!
//! Follows Clean Architecture Interface Adapters layer:
//! Safely fetches external HTML/text articles, enforces strict SSRF checks at
//! each redirect, computes SHA-256 signatures of actual saved bytes, and
//! stores content-addressed snapshots in `.state/evidence/`.

use crate::domain::{current_utc_timestamp, PrimarySourceEvidence};
use crate::ports::SourceCapturePort;
use crate::url::{is_restricted_ipv4, is_restricted_ipv6};
use sha2::{Digest, Sha256};
use std::fs::{self, File};
use std::io::{Read, Write};
use std::net::{Ipv4Addr, Ipv6Addr, ToSocketAddrs};
use std::path::{Path, PathBuf};
use std::time::Duration;
use ureq::config::Config;
use ureq::http::Uri;
use ureq::unversioned::resolver::{DefaultResolver, ResolvedSocketAddrs, Resolver};
use ureq::unversioned::transport::{DefaultConnector, NextTimeout};

pub const DEFAULT_MAX_CAPTURE_BYTES: usize = 1_048_576; // 1 MB
pub const DEFAULT_CAPTURE_TIMEOUT_SECS: u64 = 5;
pub const MAX_REDIRECT_DEPTH: usize = 3;
pub const USER_AGENT: &str = concat!("ToolcaseGateway-DailyNews/", env!("CARGO_PKG_VERSION"));

#[derive(Debug, Clone)]
pub struct HttpSourceCaptureAdapter {
    evidence_dir: PathBuf,
    max_bytes: usize,
    timeout: Duration,
    allow_http_for_tests: bool,
}

impl HttpSourceCaptureAdapter {
    pub fn new(base_dir: &Path) -> Self {
        let evidence_dir = base_dir.join(".state").join("evidence");
        Self {
            evidence_dir,
            max_bytes: DEFAULT_MAX_CAPTURE_BYTES,
            timeout: Duration::from_secs(DEFAULT_CAPTURE_TIMEOUT_SECS),
            allow_http_for_tests: false,
        }
    }

    pub fn with_allow_http(mut self, allow: bool) -> Self { self.allow_http_for_tests = allow; self }
    pub fn with_max_bytes(mut self, max: usize) -> Self { self.max_bytes = max; self }
    pub fn with_timeout(mut self, timeout: Duration) -> Self { self.timeout = timeout; self }

    pub fn is_safe_capture_url(&self, url: &str) -> Result<(), String> {
        let trimmed = url.trim();
        let after_scheme = if let Some(stripped) = trimmed.strip_prefix("https://") {
            stripped
        } else if self.allow_http_for_tests && trimmed.starts_with("http://") {
            &trimmed[7..]
        } else {
            return Err("Capture URL violates security boundary: must be HTTPS".to_string());
        };

        let host_part = after_scheme
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("")
            .trim();

        if host_part.contains('@') {
            return Err("Capture target blocked: userinfo (@) is forbidden in URL".to_string());
        }

        let host = if host_part.starts_with('[') {
            host_part.find(']').map_or_else(
                || host_part.to_lowercase(),
                |end| host_part[1..end].to_lowercase(),
            )
        } else {
            host_part.split(':').next().unwrap_or("").to_lowercase()
        };

        if host.is_empty() {
            return Err("Capture URL host cannot be empty".to_string());
        }

        let lower_host = host.to_lowercase();
        if !self.allow_http_for_tests
            && (lower_host == "localhost"
                || lower_host.ends_with(".localhost")
                || lower_host.ends_with(".local")
                || lower_host.ends_with(".internal"))
        {
            return Err(format!("Capture target blocked: local host name '{host}'"));
        }

        if let Ok(num) = host.parse::<u32>() {
            return is_restricted_ipv4(Ipv4Addr::from(num), self.allow_http_for_tests);
        }
        if let Ok(ip) = host.parse::<Ipv4Addr>() {
            return is_restricted_ipv4(ip, self.allow_http_for_tests);
        }

        let clean_ipv6 = host.trim_start_matches('[').trim_end_matches(']');
        if let Ok(ip) = clean_ipv6.parse::<Ipv6Addr>() {
            return is_restricted_ipv6(ip, self.allow_http_for_tests);
        }

        let addr_str = format!("{host}:443");
        if let Ok(addrs) = addr_str.to_socket_addrs() {
            for addr in addrs {
                match addr.ip() {
                    std::net::IpAddr::V4(v4) => is_restricted_ipv4(v4, self.allow_http_for_tests)?,
                    std::net::IpAddr::V6(v6) => is_restricted_ipv6(v6, self.allow_http_for_tests)?,
                }
            }
        }

        Ok(())
    }

    fn ensure_evidence_dir(&self) -> Result<(), String> {
        if !self.evidence_dir.exists() {
            fs::create_dir_all(&self.evidence_dir)
                .map_err(|e| format!("Failed to create evidence dir {:?}: {e}", self.evidence_dir))?;
        }
        Ok(())
    }

    fn store_snapshot_bytes(&self, bytes: &[u8]) -> Result<(String, String), String> {
        self.ensure_evidence_dir()?;
        let sha256_hex = Sha256::digest(bytes)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect::<String>();

        let filename = format!("{sha256_hex}.txt");
        let path = self.evidence_dir.join(&filename);
        if !path.exists() {
            let tmp = self.evidence_dir.join(format!("{sha256_hex}.{}.tmp", std::process::id()));
            let mut file = File::create(&tmp)
                .map_err(|e| format!("Failed to create snapshot file {tmp:?}: {e}"))?;
            file.write_all(bytes)
                .and_then(|_| file.sync_all())
                .map_err(|e| {
                    let _ = fs::remove_file(&tmp);
                    format!("Failed to write snapshot bytes: {e}")
                })?;
            fs::rename(&tmp, &path).map_err(|e| {
                let _ = fs::remove_file(&tmp);
                format!("Failed to finalize snapshot {path:?}: {e}")
            })?;
        }
        Ok((format!(".state/evidence/{filename}"), sha256_hex))
    }
}

pub fn extract_article_title(html: &str) -> Option<String> {
    let lower = html.to_ascii_lowercase();
    let start_idx = lower.find("<title")?;
    let tag_end = lower[start_idx..].find('>')?;
    let content_start = start_idx + tag_end + 1;
    let end_idx = lower[content_start..].find("</title>")?;
    let raw_title = &html[content_start..content_start + end_idx];
    let cleaned = clean_html_entities(raw_title.trim());
    (!cleaned.is_empty()).then_some(cleaned)
}

pub fn clean_html_entities(text: &str) -> String {
    text.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#39;", "'")
        .replace("&apos;", "'")
        .replace("&nbsp;", " ")
}

fn is_block_tag(t: &str) -> bool {
    matches!(
        t,
        "p" | "div" | "br" | "li" | "tr" | "td" | "article" | "section" | "blockquote"
    ) || (t.len() == 2 && t.starts_with('h') && t.ends_with(|c: char| ('1'..='6').contains(&c)))
}

pub fn extract_main_text(html: &str) -> String {
    let mut cleaned = String::with_capacity(html.len());
    let mut in_script = false;
    let mut in_style = false;
    let mut in_tag = false;
    let mut tag_buf = String::with_capacity(32);

    for c in html.chars() {
        if c == '<' {
            in_tag = true;
            tag_buf.clear();
        } else if c == '>' && in_tag {
            in_tag = false;
            let tag_lower = tag_buf.to_ascii_lowercase();
            let tag_name = tag_lower
                .trim_start_matches('/')
                .split_whitespace()
                .next()
                .unwrap_or("");
            if tag_name == "script" {
                in_script = !tag_lower.starts_with('/');
            } else if tag_name == "style" {
                in_style = !tag_lower.starts_with('/');
            } else if is_block_tag(tag_name) {
                cleaned.push(' ');
            }
        } else if in_tag {
            tag_buf.push(c);
        } else if !in_script && !in_style {
            cleaned.push(c);
        }
    }

    let decoded = clean_html_entities(&cleaned);
    decoded.split_whitespace().collect::<Vec<&str>>().join(" ")
}

#[derive(Debug, Clone)]
struct SsrfSafeResolver {
    allow_loopback: bool,
}

impl Resolver for SsrfSafeResolver {
    fn resolve(
        &self,
        uri: &Uri,
        _config: &Config,
        _timeout: NextTimeout,
    ) -> Result<ResolvedSocketAddrs, ureq::Error> {
        let scheme = uri.scheme().ok_or(ureq::Error::HostNotFound)?;
        let authority = uri.authority().ok_or(ureq::Error::HostNotFound)?;
        let addr =
            DefaultResolver::host_and_port(scheme, authority).ok_or(ureq::Error::HostNotFound)?;

        let iter = addr.to_socket_addrs().map_err(ureq::Error::Io)?;
        let mut result = self.empty();
        for socket_addr in iter {
            let check = match socket_addr.ip() {
                std::net::IpAddr::V4(v4) => is_restricted_ipv4(v4, self.allow_loopback),
                std::net::IpAddr::V6(v6) => is_restricted_ipv6(v6, self.allow_loopback),
            };
            if check.is_err() {
                return Err(ureq::Error::HostNotFound);
            }
            if result.len() < 16 {
                result.push(socket_addr);
            }
        }
        if result.is_empty() {
            return Err(ureq::Error::HostNotFound);
        }
        Ok(result)
    }
}

fn resolve_redirect_url(current_url: &str, loc: &str) -> Result<String, String> {
    if loc.starts_with("http://") || loc.starts_with("https://") {
        Ok(loc.to_string())
    } else if loc.starts_with('/') {
        let authority = current_url
            .split_once("://")
            .map(|(_, rest)| rest)
            .unwrap_or("")
            .split(['/', '?', '#'])
            .next()
            .unwrap_or("");
        let scheme = if current_url.starts_with("https://") {
            "https://"
        } else {
            "http://"
        };
        Ok(format!("{scheme}{authority}{loc}"))
    } else {
        Err(format!("INVALID_REDIRECT_LOCATION: {loc}"))
    }
}

impl HttpSourceCaptureAdapter {
    fn build_client(&self) -> ureq::Agent {
        let mut builder = ureq::Agent::config_builder()
            .proxy(None)
            .max_redirects(0)
            .timeout_global(Some(self.timeout));
        if !self.allow_http_for_tests {
            builder = builder.tls_config(
                ureq::tls::TlsConfig::builder()
                    .root_certs(ureq::tls::RootCerts::PlatformVerifier)
                    .build(),
            );
        }
        let config = builder.build();
        let resolver = SsrfSafeResolver {
            allow_loopback: self.allow_http_for_tests,
        };
        ureq::Agent::with_parts(config, DefaultConnector::default(), resolver)
    }
}

impl SourceCapturePort for HttpSourceCaptureAdapter {
    fn capture_source(&self, url: &str) -> Result<PrimarySourceEvidence, String> {
        self.is_safe_capture_url(url)?;

        let agent = self.build_client();
        let mut current_url = url.to_string();
        let mut redirects_followed = 0;

        loop {
            let res = match agent
                .get(&current_url)
                .header("User-Agent", USER_AGENT)
                .header("Accept", "text/html, text/plain, application/json")
                .call()
            {
                Ok(r) => r,
                Err(e) => {
                    return Ok(PrimarySourceEvidence {
                        url: current_url,
                        fetch_status: format!("HTTP_ERROR: {e}"),
                        ..Default::default()
                    });
                }
            };

            let status = res.status().as_u16();
            if (301..=308).contains(&status) {
                redirects_followed += 1;
                if redirects_followed > MAX_REDIRECT_DEPTH {
                    return Ok(PrimarySourceEvidence {
                        url: current_url,
                        fetch_status: "TOO_MANY_REDIRECTS".to_string(),
                        ..Default::default()
                    });
                }

                let Some(loc) = res.headers().get("location").and_then(|v| v.to_str().ok()) else {
                    return Ok(PrimarySourceEvidence {
                        url: current_url,
                        fetch_status: "REDIRECT_MISSING_LOCATION".to_string(),
                        ..Default::default()
                    });
                };

                let next_url = match resolve_redirect_url(&current_url, loc) {
                    Ok(u) => u,
                    Err(err_msg) => {
                        return Ok(PrimarySourceEvidence {
                            url: current_url,
                            fetch_status: err_msg,
                            ..Default::default()
                        });
                    }
                };

                // Re-validate SSRF on redirect target!
                if let Err(e) = self.is_safe_capture_url(&next_url) {
                    return Ok(PrimarySourceEvidence {
                        url: next_url,
                        fetch_status: format!("SSRF_REDIRECT_BLOCKED: {e}"),
                        ..Default::default()
                    });
                }
                current_url = next_url;
                continue;
            }

            let content_type = res
                .headers()
                .get("content-type")
                .and_then(|v| v.to_str().ok())
                .unwrap_or("text/html")
                .to_string();

            let ct_lower = content_type.to_lowercase();
            if !ct_lower.contains("text/html")
                && !ct_lower.contains("text/plain")
                && !ct_lower.contains("application/json")
            {
                return Ok(PrimarySourceEvidence {
                    url: current_url,
                    fetch_status: format!("UNSUPPORTED_MIME_TYPE: {content_type}"),
                    content_type: Some(content_type),
                    ..Default::default()
                });
            }

            let mut reader = res
                .into_body()
                .into_reader()
                .take((self.max_bytes + 1) as u64);
            let mut bytes = Vec::new();
            if let Err(e) = reader.read_to_end(&mut bytes) {
                return Ok(PrimarySourceEvidence {
                    url: current_url,
                    fetch_status: format!("READ_ERROR: {e}"),
                    content_type: Some(content_type),
                    ..Default::default()
                });
            }

            if bytes.len() > self.max_bytes {
                return Ok(PrimarySourceEvidence {
                    url: current_url,
                    fetch_status: format!("PAYLOAD_TOO_LARGE: exceeded {} byte limit", self.max_bytes),
                    content_type: Some(content_type),
                    ..Default::default()
                });
            }

            let (snapshot_ref, snapshot_sha256) = self.store_snapshot_bytes(&bytes)?;
            let text_content = String::from_utf8_lossy(&bytes).to_string();
            let page_title = extract_article_title(&text_content);
            let main_text = extract_main_text(&text_content);
            let excerpt: String = main_text.chars().take(500).collect();

            let (fetch_status, content_quality) = match crate::content_quality::evaluate_content_quality_with_source(
                page_title.as_deref(),
                &text_content,
                &main_text,
                Some(&current_url),
            ) {
                Ok(q) => ("SUCCESS".to_string(), Some(q)),
                Err(e) => (format!("QUALITY_GATE_FAILED: {e}"), Some("QUALITY_GATE_FAILED".to_string())),
            };

            return Ok(PrimarySourceEvidence {
                url: current_url,
                fetch_status,
                content_quality,
                content_type: Some(content_type),
                title: page_title,
                published_at: None,
                excerpt: Some(excerpt),
                snapshot_ref: Some(snapshot_ref),
                snapshot_sha256: Some(snapshot_sha256),
                captured_at: Some(current_utc_timestamp()),
                extraction_method: Some("conservative_html".to_string()),
            });
        }
    }

    fn read_snapshot_text(&self, snapshot_ref: &str) -> Option<String> {
        let hash = snapshot_ref.trim_end_matches(".txt").split('/').next_back()?;
        let path = self.evidence_dir.join(format!("{hash}.txt"));
        fs::read_to_string(path).ok()
    }
}
