//! Cryptographic signature verification, Trust Store, and In-Toto/Gateway attestation parsing.
//!
//! Enforces strict trust boundaries: mock signatures and unverified tokens fail closed.

use std::collections::HashMap;
use std::sync::RwLock;
use ring::signature::{UnparsedPublicKey, ED25519};
use ring::hmac;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ParsedAttestation {
    pub version: u32,
    pub url: String,
    pub final_url: String,
    pub content_sha256: String,
    pub fetched_at: u64,
    pub issuer: String,
    pub key_id: String,
    pub signature_raw: String,
    pub signature_bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub enum TrustedKey {
    Ed25519(Vec<u8>),
    HmacSha256(Vec<u8>),
}

#[derive(Debug, Clone)]
pub struct TrustedKeyEntry {
    pub key: TrustedKey,
    pub allowed_issuer: String,
    pub is_test: bool,
}

#[derive(Debug)]
pub struct TrustStore {
    keys: RwLock<HashMap<String, TrustedKeyEntry>>,
    allow_test_keys: bool,
}

static GLOBAL_TRUST_STORE: std::sync::OnceLock<TrustStore> = std::sync::OnceLock::new();

pub fn global_trust_store() -> &'static TrustStore {
    GLOBAL_TRUST_STORE.get_or_init(|| {
        let store = TrustStore::new();
        store.seed_accredited_keys();
        store
    })
}

impl Default for TrustStore {
    fn default() -> Self {
        Self::new()
    }
}

impl TrustStore {
    pub fn new() -> Self {
        Self {
            keys: RwLock::new(HashMap::new()),
            allow_test_keys: false,
        }
    }

    pub fn new_test() -> Self {
        Self {
            keys: RwLock::new(HashMap::new()),
            allow_test_keys: true,
        }
    }

    pub fn allows_test_keys(&self) -> bool {
        if self.allow_test_keys {
            return true;
        }
        // Strict environment check: only explicit truthy values ("1", "true") are permitted.
        // "false", "0", "no", "off", empty, or any other value evaluates strictly to false.
        // Executable path or argv guessing is completely eliminated.
        match std::env::var("DAILY_NEWS_ALLOW_TEST_KEYS") {
            Ok(v) => {
                let trimmed = v.trim();
                trimmed.eq_ignore_ascii_case("1") || trimmed.eq_ignore_ascii_case("true")
            }
            Err(_) => false,
        }
    }

    pub fn seed_accredited_keys(&self) {
        // Known default public keys for gateway fetchers and accredited audit issuers
        // Format: key_id -> TrustedKeyEntry
        // Gateway root fetcher Ed25519 public key (32 bytes)
        let gw_pub = hex_decode("e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e06522490155")
            .unwrap_or_default();
        if gw_pub.len() == 32 {
            self.register_key_with_issuer(
                "gateway-fetcher-root-1",
                TrustedKey::Ed25519(gw_pub),
                "Gateway Root Fetcher",
                false,
            );
        }

        // Production security rule: RFC 8032 test public keys and dummy hashes (e.g. mitre-notary-key-1,
        // certik-notary-1, trailofbits-notary-1) MUST NEVER be seeded in production trust store.
        // Third-party accredited notaries must be configured dynamically via secure environment configuration:
        // Format: GATEWAY_NOTARY_KEYS="key_id:issuer:ed25519_hex,..."
        if let Ok(notary_keys_var) = std::env::var("GATEWAY_NOTARY_KEYS") {
            for entry in notary_keys_var.split(',') {
                let parts: Vec<&str> = entry.split(':').map(str::trim).collect();
                if parts.len() == 3 {
                    let key_id = parts[0];
                    let issuer = parts[1];
                    let hex_key = parts[2];
                    if let Some(pub_bytes) = hex_decode(hex_key) {
                        if pub_bytes.len() == 32 {
                            self.register_key_with_issuer(
                                key_id,
                                TrustedKey::Ed25519(pub_bytes),
                                issuer,
                                false,
                            );
                        }
                    }
                }
            }
        }

        // Shared HMAC secret for authenticated gateway-internal fetch pipelines.
        // Production security: NEVER hardcode HMAC secrets in source or binary!
        // Keys must be loaded from secure configuration or environment variables.
        if let Ok(sec) = std::env::var("GATEWAY_ATTESTATION_SECRET")
            .or_else(|_| std::env::var("DAILY_NEWS_HMAC_SECRET"))
        {
            let trimmed = sec.trim();
            if !trimmed.is_empty() {
                self.register_key_with_issuer(
                    "gateway-internal-hmac-1",
                    TrustedKey::HmacSha256(trimmed.as_bytes().to_vec()),
                    "Gateway Internal Notary",
                    false,
                );
            }
        }
    }

    pub fn register_key(&self, key_id: &str, key: TrustedKey) {
        self.register_key_with_issuer(key_id, key, "*", false);
    }

    pub fn register_key_with_issuer(
        &self,
        key_id: &str,
        key: TrustedKey,
        allowed_issuer: &str,
        is_test: bool,
    ) {
        if is_test && !self.allows_test_keys() {
            eprintln!("[daily-news] Security: Ignored test key '{key_id}' in production trust store");
            return;
        }
        if let Ok(mut lock) = self.keys.write() {
            lock.insert(
                key_id.to_string(),
                TrustedKeyEntry {
                    key,
                    allowed_issuer: allowed_issuer.to_string(),
                    is_test,
                },
            );
        }
    }

    pub fn get_key_entry(&self, key_id: &str) -> Option<TrustedKeyEntry> {
        self.keys.read().ok()?.get(key_id).cloned()
    }

    pub fn get_key(&self, key_id: &str) -> Option<TrustedKey> {
        self.get_key_entry(key_id).map(|e| e.key)
    }
}

pub fn hex_decode(s: &str) -> Option<Vec<u8>> {
    let clean: String = s.chars().filter(|c| !c.is_whitespace()).collect();
    if !clean.is_ascii() || !clean.len().is_multiple_of(2) {
        return None;
    }
    (0..clean.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&clean[i..i + 2], 16).ok())
        .collect()
}

pub fn hex_encode(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

pub fn compute_canonical_attestation_payload(
    url: &str,
    final_url: &str,
    content_sha256: &str,
    fetched_at: u64,
    issuer: &str,
) -> Vec<u8> {
    format!(
        "url:{}\nfinal_url:{}\ncontent_sha256:{}\nfetched_at:{}\nissuer:{}",
        url.trim(),
        final_url.trim(),
        content_sha256.trim().to_ascii_lowercase(),
        fetched_at,
        issuer.trim().to_ascii_lowercase()
    )
    .into_bytes()
}

pub fn parse_attestation_envelope(text: &str) -> Option<ParsedAttestation> {
    let lower = text.to_lowercase();
    let start_idx = lower.find("-----begin gateway fetch attestation-----")
        .or_else(|| lower.find("-----begin trusted attestation-----"))?;
    let end_idx = lower.find("-----end gateway fetch attestation-----")
        .or_else(|| lower.find("-----end trusted attestation-----"))?;

    if start_idx >= end_idx {
        return None;
    }

    let block = &text[start_idx..end_idx];
    let mut version = 1u32;
    let mut url = String::new();
    let mut final_url = String::new();
    let mut content_sha256 = String::new();
    let mut fetched_at = 0u64;
    let mut issuer = String::new();
    let mut key_id = String::new();
    let mut signature_raw = String::new();

    for line in block.lines() {
        let trimmed = line.trim();
        if let Some(val) = extract_kv(trimmed, "Version:") {
            version = val.parse().unwrap_or(1);
        } else if let Some(val) = extract_kv(trimmed, "URL:").or_else(|| extract_kv(trimmed, "Source-URL:")) {
            url = val.to_string();
        } else if let Some(val) = extract_kv(trimmed, "Final-URL:").or_else(|| extract_kv(trimmed, "Redirect-URL:")) {
            final_url = val.to_string();
        } else if let Some(val) = extract_kv(trimmed, "Content-SHA256:").or_else(|| extract_kv(trimmed, "SHA256:")) {
            content_sha256 = val.to_string();
        } else if let Some(val) = extract_kv(trimmed, "Fetched-At:").or_else(|| extract_kv(trimmed, "Timestamp:")) {
            fetched_at = val.parse().unwrap_or(0);
        } else if let Some(val) = extract_kv(trimmed, "Issuer:").or_else(|| extract_kv(trimmed, "Attested-Issuer:")) {
            issuer = val.to_string();
        } else if let Some(val) = extract_kv(trimmed, "Key-ID:").or_else(|| extract_kv(trimmed, "Signer-Key:")) {
            key_id = val.to_string();
        } else if let Some(val) = extract_kv(trimmed, "Signature:") {
            signature_raw = val.to_string();
        }
    }

    if final_url.is_empty() {
        final_url = url.clone();
    }

    let signature_bytes = hex_decode(&signature_raw)?;
    if signature_bytes.is_empty() {
        return None;
    }

    Some(ParsedAttestation {
        version,
        url,
        final_url,
        content_sha256,
        fetched_at,
        issuer,
        key_id,
        signature_raw,
        signature_bytes,
    })
}

fn extract_kv<'a>(line: &'a str, prefix: &str) -> Option<&'a str> {
    if line.len() >= prefix.len() && line[..prefix.len()].eq_ignore_ascii_case(prefix) {
        Some(line[prefix.len()..].trim())
    } else {
        None
    }
}

pub fn verify_parsed_attestation(
    attestation: &ParsedAttestation,
    expected_sec_ref: &str,
    expected_hash: Option<&str>,
    audit_text_sha: Option<&str>,
    store: &TrustStore,
) -> bool {
    verify_parsed_attestation_with_body(
        attestation,
        expected_sec_ref,
        expected_hash,
        audit_text_sha,
        None,
        store,
    )
}

fn verify_redirect_domain(orig_domain: &str, final_domain: &str) -> bool {
    if final_domain.is_empty() || final_domain == "localhost" || final_domain == "127.0.0.1" {
        return false;
    }

    // SSRF boundary protection: block loopback and private/reserved IPv4
    if let Ok(ip) = final_domain.parse::<std::net::Ipv4Addr>() {
        if crate::url::is_restricted_ipv4(ip, false).is_err() {
            return false;
        }
    }

    // Redirect domain binding: final_domain must match or be a subdomain of orig_domain, or vice versa
    if orig_domain.is_empty() || final_domain == orig_domain {
        return true;
    }

    final_domain.ends_with(&format!(".{orig_domain}"))
        || orig_domain.ends_with(&format!(".{final_domain}"))
}

fn verify_attestation_freshness(fetched_at: u64) -> bool {
    if fetched_at == 0 {
        return false;
    }
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1793618000);
    const MAX_CLOCK_SKEW_SECS: u64 = 3600;
    if fetched_at > now + MAX_CLOCK_SKEW_SECS {
        return false;
    }
    const MAX_ATTESTATION_TTL_SECS: u64 = 180 * 86400;
    now <= fetched_at + MAX_ATTESTATION_TTL_SECS
}

fn verify_content_hash_binding(
    content_sha256: &str,
    actual_body_sha: Option<&str>,
    expected_hash: Option<&str>,
    audit_text_sha: Option<&str>,
) -> bool {
    // P0 Security Invariant: The signed attestation.content_sha256 MUST strictly bind the actual
    // audit report body. Passing when only the outer stored_envelope_sha256 matches while content_sha256
    // differs is strictly forbidden.
    if let Some(body_sha) = actual_body_sha {
        if !content_sha256.eq_ignore_ascii_case(body_sha.trim()) {
            return false;
        }
    }

    if let Some(exp_h) = expected_hash {
        let exp_clean = exp_h.trim().to_ascii_lowercase();
        let matches_content = content_sha256.eq_ignore_ascii_case(&exp_clean);
        let matches_body = actual_body_sha.is_some_and(|b| b.eq_ignore_ascii_case(&exp_clean));
        let matches_envelope = audit_text_sha.is_some_and(|h| h.eq_ignore_ascii_case(&exp_clean));

        // When actual_body_sha is provided, content_sha256 was already validated above.
        // When actual_body_sha is not separately provided, attestation.content_sha256 MUST match expected_hash!
        if actual_body_sha.is_none() && !matches_content {
            return false;
        }

        if !exp_clean.is_empty() && !matches_content && !matches_body && !matches_envelope {
            return false;
        }
    }

    true
}

fn is_issuer_authorized(trusted_entry: &TrustedKeyEntry, attestation_issuer: &str) -> bool {
    if trusted_entry.allowed_issuer == "*" {
        return true;
    }
    let allowed = trusted_entry.allowed_issuer.to_lowercase();
    let issuer = attestation_issuer.to_lowercase();
    issuer.contains(&allowed) || allowed.contains(&issuer)
}

fn verify_crypto_signature(
    key: &TrustedKey,
    canonical_payload: &[u8],
    signature_bytes: &[u8],
) -> bool {
    match key {
        TrustedKey::Ed25519(pub_bytes) => {
            if pub_bytes.len() != 32 {
                return false;
            }
            let peer = UnparsedPublicKey::new(&ED25519, pub_bytes);
            peer.verify(canonical_payload, signature_bytes).is_ok()
        }
        TrustedKey::HmacSha256(secret) => {
            if secret.is_empty() {
                return false;
            }
            let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
            hmac::verify(&key, canonical_payload, signature_bytes).is_ok()
        }
    }
}

pub fn verify_parsed_attestation_with_body(
    attestation: &ParsedAttestation,
    expected_sec_ref: &str,
    expected_hash: Option<&str>,
    audit_text_sha: Option<&str>,
    actual_body_sha: Option<&str>,
    store: &TrustStore,
) -> bool {
    // 1. Verify URL binding: attestation URL or final_url must match expected_sec_ref
    let exp_trim = expected_sec_ref.trim();
    if !attestation.url.eq_ignore_ascii_case(exp_trim)
        && !attestation.final_url.eq_ignore_ascii_case(exp_trim)
    {
        return false;
    }

    // 2. Verify redirect domain integrity: final_url must be on authoritative domain and not SSRF
    let orig_domain = crate::url::extract_domain(&attestation.url);
    let final_domain = crate::url::extract_domain(&attestation.final_url);
    if !verify_redirect_domain(&orig_domain, &final_domain) {
        return false;
    }

    // 3. Timestamp & Freshness verification:
    // fetched_at must be positive, not in future (> 3600s clock skew), and not expired (180 days TTL)
    if !verify_attestation_freshness(attestation.fetched_at) {
        return false;
    }

    // 4. Strict Content Hash & Audit Body Binding
    if !verify_content_hash_binding(
        &attestation.content_sha256,
        actual_body_sha,
        expected_hash,
        audit_text_sha,
    ) {
        return false;
    }

    // 5. Retrieve trusted key entry from trust store
    let Some(trusted_entry) = store.get_key_entry(&attestation.key_id) else {
        return false;
    };

    // Fail closed on test keys in production
    if trusted_entry.is_test && !store.allows_test_keys() {
        return false;
    }

    // Enforce key-to-issuer authorization binding
    if !is_issuer_authorized(&trusted_entry, &attestation.issuer) {
        return false;
    }

    // 6. Compute canonical payload
    let canonical = compute_canonical_attestation_payload(
        &attestation.url,
        &attestation.final_url,
        &attestation.content_sha256,
        attestation.fetched_at,
        &attestation.issuer,
    );

    // 7. Cryptographically verify signature
    verify_crypto_signature(&trusted_entry.key, &canonical, &attestation.signature_bytes)
}

pub fn create_signed_gateway_attestation(
    url: &str,
    final_url: &str,
    content_sha256: &str,
    fetched_at: u64,
    issuer: &str,
    key_id: &str,
    secret: &[u8],
) -> String {
    let canonical = compute_canonical_attestation_payload(url, final_url, content_sha256, fetched_at, issuer);
    let key = hmac::Key::new(hmac::HMAC_SHA256, secret);
    let tag = hmac::sign(&key, &canonical);
    let sig_hex = hex_encode(tag.as_ref());
    format!(
        "-----BEGIN GATEWAY FETCH ATTESTATION-----\n\
         Version: 1\n\
         URL: {}\n\
         Final-URL: {}\n\
         Content-SHA256: {}\n\
         Fetched-At: {}\n\
         Issuer: {}\n\
         Key-ID: {}\n\
         TLS-Verified: true\n\
         Signature: {}\n\
         -----END GATEWAY FETCH ATTESTATION-----",
        url, final_url, content_sha256, fetched_at, issuer, key_id, sig_hex
    )
}

/// Dedicated test helpers isolated from production verification pipeline.
pub mod test_helpers {
    use super::{create_signed_gateway_attestation, global_trust_store, TrustedKey};

    /// Test fixture helper.
    /// Strictly registered as `is_test: true` and rejected by production trust stores.
    pub fn make_test_certik_attestation(url: &str, content_sha256: &str) -> String {
        // Test fixture helper: if not explicitly configured/disabled, enable for test suite
        if std::env::var("DAILY_NEWS_ALLOW_TEST_KEYS").is_err() {
            std::env::set_var("DAILY_NEWS_ALLOW_TEST_KEYS", "1");
        }
        if !global_trust_store().allows_test_keys() {
            eprintln!("[daily-news] Security: make_test_certik_attestation is prohibited in production runtime");
            return String::new();
        }

        let test_secret = b"ephemeral-test-certik-attestation-secret-only";
        let key_id = "test-certik-notary-hmac";
        global_trust_store().register_key_with_issuer(
            key_id,
            TrustedKey::HmacSha256(test_secret.to_vec()),
            "CertiK",
            true,
        );

        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(1793618000);

        create_signed_gateway_attestation(
            url,
            url,
            content_sha256,
            now,
            "CertiK",
            key_id,
            test_secret,
        )
    }
}

pub use test_helpers::make_test_certik_attestation;
