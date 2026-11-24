//! Domain entities and value objects for upstream proxies, routing strategies, and authentication.
//!
//! Clean Architecture: Core Domain Layer.
//! Pure business rules with zero transport, framework, or database dependencies.

use serde::{Deserialize, Serialize};

/// Upstream proxy protocol.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProxyProtocol {
    Http,
    Https,
    Socks5,
}

impl ProxyProtocol {
    pub fn parse(s: &str) -> Self {
        match s.to_ascii_lowercase().as_str() {
            "socks5" | "socks5h" => Self::Socks5,
            "https" => Self::Https,
            _ => Self::Http,
        }
    }

    pub fn scheme(&self) -> &'static str {
        match self {
            Self::Http => "http",
            Self::Https => "https",
            Self::Socks5 => "socks5",
        }
    }
}

/// Upstream proxy authentication credentials.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProxyAuth {
    pub username: String,
    pub password: String,
}

/// Upstream proxy configuration and health metadata.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UpstreamProxy {
    pub id: String,
    pub name: String,
    pub url: String,
    pub protocol: ProxyProtocol,
    pub host: String,
    pub port: u16,
    pub auth: Option<ProxyAuth>,
    pub country: Option<String>,
    pub weight: u32,
    pub enabled: bool,
    pub failure_count: u32,
    pub success_count: u64,
    pub tags: Vec<String>,
}

impl UpstreamProxy {
    /// Parses an upstream proxy definition string into a structured UpstreamProxy entity.
    ///
    /// Supported formats:
    /// - `http://user:pass@host:port`
    /// - `socks5://host:port`
    /// - `host:port:user:pass` (standard proxy vendor format)
    /// - `host:port`
    pub fn parse(
        raw_url: &str,
        name: Option<&str>,
        country: Option<&str>,
        tags: Option<Vec<String>>,
    ) -> Result<Self, String> {
        let trimmed = raw_url.trim();
        if trimmed.is_empty() {
            return Err("Proxy URL cannot be empty".to_string());
        }

        let (protocol, host, port, auth) = parse_proxy_components(trimmed)?;

        let id = format!("{}:{}", host, port);
        let proxy_name = name.unwrap_or(&id).to_string();

        let canonical_url = match &auth {
            Some(a) => format!(
                "{}://{}:{}@{}:{}",
                protocol.scheme(),
                url_encode_userinfo(&a.username),
                url_encode_userinfo(&a.password),
                host,
                port
            ),
            None => format!("{}://{}:{}", protocol.scheme(), host, port),
        };

        Ok(Self {
            id,
            name: proxy_name,
            url: canonical_url,
            protocol,
            host,
            port,
            auth,
            country: country.map(|c| c.trim().to_ascii_uppercase()),
            weight: 1,
            enabled: true,
            failure_count: 0,
            success_count: 0,
            tags: tags.unwrap_or_default(),
        })
    }

    /// Returns the canonical proxy URL with passwords masked (safe for logging & MCP outputs).
    pub fn masked_url(&self) -> String {
        match &self.auth {
            Some(a) => format!(
                "{}://{}:***@{}:{}",
                self.protocol.scheme(),
                url_encode_userinfo(&a.username),
                self.host,
                self.port
            ),
            None => format!("{}://{}:{}", self.protocol.scheme(), self.host, self.port),
        }
    }

    /// Returns a copy of the proxy entity with credentials masked for safe serialization.
    pub fn to_masked(&self) -> Self {
        let mut cloned = self.clone();
        cloned.url = self.masked_url();
        if let Some(ref mut a) = cloned.auth {
            a.password = "***".to_string();
        }
        cloned
    }

    /// Formats this proxy into a standard format for client tools (Playwright, Puppeteer).
    pub fn to_playwright_config(&self) -> PlaywrightProxyConfig {
        PlaywrightProxyConfig {
            server: format!("{}://{}:{}", self.protocol.scheme(), self.host, self.port),
            username: self.auth.as_ref().map(|a| a.username.clone()),
            password: self.auth.as_ref().map(|a| a.password.clone()),
        }
    }
}

/// Helper to parse URL or vendor string `host:port:user:pass`
fn parse_proxy_components(
    s: &str,
) -> Result<(ProxyProtocol, String, u16, Option<ProxyAuth>), String> {
    if s.contains("://") {
        let parts: Vec<&str> = s.splitn(2, "://").collect();
        let protocol = ProxyProtocol::parse(parts[0]);
        let rest = parts[1];

        let (auth, host_port) = if let Some(idx) = rest.find('@') {
            let auth_part = &rest[..idx];
            let hp = &rest[idx + 1..];
            let creds: Vec<&str> = auth_part.splitn(2, ':').collect();
            let auth = if creds.len() == 2 {
                Some(ProxyAuth {
                    username: url_decode(creds[0]),
                    password: url_decode(creds[1]),
                })
            } else {
                None
            };
            (auth, hp)
        } else {
            (None, rest)
        };

        let clean_hp = host_port.split('/').next().unwrap_or(host_port);
        let (host, port) = parse_host_port(clean_hp)?;

        Ok((protocol, host, port, auth))
    } else if s.starts_with('[') {
        let (host, remainder) = if let Some(close_idx) = s.find(']') {
            (s[..=close_idx].to_string(), &s[close_idx + 1..])
        } else {
            return Err("Mismatched brackets in IPv6 address".to_string());
        };
        let rest = remainder.strip_prefix(':').unwrap_or(remainder);
        let segments: Vec<&str> = rest.split(':').collect();
        match segments.len() {
            1 => {
                let port: u16 = segments[0]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                Ok((ProxyProtocol::Http, host, port, None))
            }
            3 => {
                let port: u16 = segments[0]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                let auth = Some(ProxyAuth {
                    username: segments[1].to_string(),
                    password: segments[2].to_string(),
                });
                Ok((ProxyProtocol::Http, host, port, auth))
            }
            _ => Err(
                "Invalid IPv6 proxy format. Expected '[ipv6]:port' or '[ipv6]:port:user:pass'"
                    .to_string(),
            ),
        }
    } else {
        // Vendor format: host:port:user:pass or host:port
        let segments: Vec<&str> = s.split(':').collect();
        match segments.len() {
            2 => {
                let host = segments[0].to_string();
                let port: u16 = segments[1]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                Ok((ProxyProtocol::Http, host, port, None))
            }
            4 => {
                let host = segments[0].to_string();
                let port: u16 = segments[1]
                    .parse()
                    .map_err(|_| "Invalid proxy port".to_string())?;
                let auth = Some(ProxyAuth {
                    username: segments[2].to_string(),
                    password: segments[3].to_string(),
                });
                Ok((ProxyProtocol::Http, host, port, auth))
            }
            _ => Err(
                "Invalid proxy format. Expected 'protocol://host:port' or 'host:port:user:pass'"
                    .to_string(),
            ),
        }
    }
}

fn parse_host_port(hp: &str) -> Result<(String, u16), String> {
    if hp.starts_with('[') {
        if let Some(close_idx) = hp.find(']') {
            let host = hp[..=close_idx].to_string();
            let remainder = &hp[close_idx + 1..];
            let port = if let Some(stripped) = remainder.strip_prefix(':') {
                stripped
                    .parse::<u16>()
                    .map_err(|_| "Invalid proxy port".to_string())?
            } else {
                8080
            };
            Ok((host, port))
        } else {
            Err("Mismatched brackets in IPv6 address".to_string())
        }
    } else {
        let hp_parts: Vec<&str> = hp.split(':').collect();
        if hp_parts.is_empty() || hp_parts[0].is_empty() {
            return Err("Invalid proxy host".to_string());
        }
        let host = hp_parts[0].to_string();
        let port: u16 = if hp_parts.len() > 1 {
            hp_parts[1]
                .parse()
                .map_err(|_| "Invalid proxy port".to_string())?
        } else {
            8080
        };
        Ok((host, port))
    }
}

/// Playwright / Puppeteer browser launch proxy dictionary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlaywrightProxyConfig {
    pub server: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub username: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub password: Option<String>,
}

/// Upstream proxy rotation strategy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum ProxyRotationStrategy {
    /// Simple round-robin across enabled healthy proxies.
    #[default]
    RoundRobin,
    /// Random selection from pool.
    Random,
    /// Sticky session: same session key routes to the same proxy until failure.
    StickySession,
    /// Selects proxy with the lowest failure count.
    LowestFailures,
}

/// Query criteria when selecting an upstream proxy from the pool.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ProxySelectionQuery {
    pub session_id: Option<String>,
    pub country: Option<String>,
    pub tag: Option<String>,
    pub preferred_id: Option<String>,
}

fn url_decode(s: &str) -> String {
    let mut decoded = Vec::with_capacity(s.len());
    let mut bytes = s.as_bytes().iter();
    while let Some(&b) = bytes.next() {
        if b == b'%' {
            if let (Some(&h1), Some(&h2)) = (bytes.next(), bytes.next()) {
                let hex_str = [h1, h2];
                if let Ok(hex) = std::str::from_utf8(&hex_str) {
                    if let Ok(byte) = u8::from_str_radix(hex, 16) {
                        decoded.push(byte);
                        continue;
                    }
                }
                decoded.push(b'%');
                decoded.push(h1);
                decoded.push(h2);
            } else {
                decoded.push(b'%');
            }
        } else {
            decoded.push(b);
        }
    }
    String::from_utf8(decoded).unwrap_or_else(|_| s.to_string())
}

fn url_encode_userinfo(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'a'..=b'z' | b'A'..=b'Z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char);
            }
            _ => {
                out.push('%');
                const HEX_CHARS: &[u8; 16] = b"0123456789ABCDEF";
                out.push(HEX_CHARS[(b >> 4) as usize] as char);
                out.push(HEX_CHARS[(b & 0x0F) as usize] as char);
            }
        }
    }
    out
}
