use std::borrow::Cow;
use std::fs;
use std::marker::PhantomData;
use std::path::PathBuf;

/// Zero-copy struct with Lifetime `'a` (0 heap allocation)
#[derive(Debug, PartialEq, Eq)]
pub struct LogEntry<'a> {
    pub level: &'a str,
    pub message: &'a str,
}

impl<'a> LogEntry<'a> {
    pub fn parse(line: &'a str) -> Option<Self> {
        let (level, message) = line.split_once(": ")?;
        Some(Self { level, message })
    }
}

/// Conditional transformation using `Cow<'a, str>`
pub fn sanitize_identifier(input: &str) -> Cow<'_, str> {
    if !input.contains(' ') {
        return Cow::Borrowed(input);
    }
    Cow::Owned(input.replace(' ', "_"))
}

/// Lazy mutation using `Cow::to_mut()`
pub fn ensure_https(url: &str) -> Cow<'_, str> {
    let mut cow = Cow::Borrowed(url);
    if !cow.starts_with("https://") {
        cow.to_mut().insert_str(0, "https://");
    }
    cow
}

/// Domain error for Newtype pattern verification
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainError {
    InvalidEmail,
}

/// Newtype pattern ("Parse, Don't Validate")
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct EmailAddress(String);

impl EmailAddress {
    pub fn parse(raw: impl Into<String>) -> Result<Self, DomainError> {
        let s = raw.into();
        if s.contains('@') && s.contains('.') {
            Ok(Self(s))
        } else {
            Err(DomainError::InvalidEmail)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Typestate pattern marker types
pub struct Disconnected;
pub struct Connected;

pub struct Connection<State> {
    endpoint: String,
    _state: PhantomData<State>,
}

impl Connection<Disconnected> {
    pub fn new(endpoint: impl Into<String>) -> Self {
        Self {
            endpoint: endpoint.into(),
            _state: PhantomData,
        }
    }

    pub fn connect(self) -> Connection<Connected> {
        Connection {
            endpoint: self.endpoint,
            _state: PhantomData,
        }
    }
}

impl Connection<Connected> {
    pub fn send_ping(&self) -> String {
        let mut out = String::with_capacity(self.endpoint.len() + 8);
        out.push_str("PING ");
        out.push_str(&self.endpoint);
        out
    }
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("Failed to resolve gateway-modules repository root")
        .to_path_buf()
}

fn assert_agent_rules_document(doc_name: &str, content: &str) {
    let required_unwrap_rules = [
        "unwrap()",
        "unwrap_or",
        "unwrap_or_else",
        "unwrap_or_default",
        "expect",
    ];
    for token in required_unwrap_rules {
        assert!(
            content.contains(token),
            "{doc_name} must document unwrap safety rule token `{token}`"
        );
    }

    let required_antipatterns = [
        "&String",
        "&Vec",
        "let _ =",
        ".enumerate()",
        ".join",
        "unsafe",
        "cargo clippy",
    ];
    for token in required_antipatterns {
        assert!(
            content.contains(token),
            "{doc_name} must document Rust anti-pattern token `{token}`"
        );
    }

    let required_lifetime_and_cow = [
        "Lifetimes",
        "Cow<'a, B>",
        "Cow::Borrowed",
        "Cow::Owned",
        ".to_mut()",
        "#[serde(borrow)]",
        "LogEntry<'a>",
    ];
    for token in required_lifetime_and_cow {
        assert!(
            content.contains(token),
            "{doc_name} must document Lifetime/Cow token `{token}`"
        );
    }

    let required_advanced_rules = [
        "Parse, Don't Validate",
        "EmailAddress",
        "Typestate",
        "thiserror",
        "anyhow",
        "with_capacity",
        "MutexGuard",
    ];
    for token in required_advanced_rules {
        assert!(
            content.contains(token),
            "{doc_name} must document Advanced AI Rust Code Generation token `{token}`"
        );
    }
}

#[test]
fn test_repository_agents_and_gemini_md_rules_completeness() {
    let root = repo_root();
    for doc_name in ["AGENTS.md", "GEMINI.md"] {
        let path = root.join(doc_name);
        assert!(path.exists(), "{} must exist at {:?}", doc_name, path);
        let content = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("Failed to read {doc_name}: {e}"));
        assert_agent_rules_document(doc_name, &content);
    }
}

#[test]
fn test_zero_copy_lifetime_log_entry() {
    let raw_line = String::from("INFO: Zero-copy borrowing verified");
    let entry = LogEntry::parse(&raw_line).expect("Valid log line must parse");
    assert_eq!(entry.level, "INFO");
    assert_eq!(entry.message, "Zero-copy borrowing verified");

    // Verify zero-copy: slice pointers lie strictly within raw_line's buffer
    let base_ptr = raw_line.as_ptr() as usize;
    let end_ptr = base_ptr + raw_line.len();
    let level_ptr = entry.level.as_ptr() as usize;
    let msg_ptr = entry.message.as_ptr() as usize;
    assert!(level_ptr >= base_ptr && level_ptr < end_ptr);
    assert!(msg_ptr >= base_ptr && msg_ptr < end_ptr);
}

#[test]
fn test_cow_conditional_sanitization_and_lazy_mutation() {
    let clean = "valid_rust_identifier";
    let sanitized_clean = sanitize_identifier(clean);
    assert!(
        matches!(sanitized_clean, Cow::Borrowed(_)),
        "Fast path without spaces must return Cow::Borrowed (0 heap allocation)"
    );
    assert_eq!(sanitized_clean, "valid_rust_identifier");

    let dirty = "needs space replacement";
    let sanitized_dirty = sanitize_identifier(dirty);
    assert!(
        matches!(sanitized_dirty, Cow::Owned(_)),
        "Slow path with spaces must allocate Cow::Owned"
    );
    assert_eq!(sanitized_dirty, "needs_space_replacement");

    let secure_url = "https://example.com/feed.xml";
    let checked_secure = ensure_https(secure_url);
    assert!(matches!(checked_secure, Cow::Borrowed(_)));
    assert_eq!(checked_secure, "https://example.com/feed.xml");

    let bare_url = "example.com/feed.xml";
    let upgraded_url = ensure_https(bare_url);
    assert!(matches!(upgraded_url, Cow::Owned(_)));
    assert_eq!(upgraded_url, "https://example.com/feed.xml");
}

#[test]
fn test_newtype_and_typestate_patterns() {
    let valid = EmailAddress::parse("agent@example.com").expect("Valid email should parse");
    assert_eq!(valid.as_str(), "agent@example.com");
    assert_eq!(
        EmailAddress::parse("invalid-email"),
        Err(DomainError::InvalidEmail)
    );

    let disconnected = Connection::<Disconnected>::new("127.0.0.1:8080");
    let connected = disconnected.connect();
    assert_eq!(connected.send_ping(), "PING 127.0.0.1:8080");
}
