//! Redis Queue & Stream Feed Provider.
//!
//! Synchronous, non-panicking RESP-compatible client over `std::net::TcpStream`.
//! Implements support for Redis Streams (XREVRANGE) and Redis Lists (LRANGE).

use crate::domain::{format_utc_timestamp, RawNewsItem};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Write};
use std::net::{TcpStream, ToSocketAddrs};
use std::time::Duration;

pub struct RedisFeedProvider {
    pub redis_url: String,
    pub key: String,
    pub channel_type: String,
    pub timeout: Duration,
}

impl RedisFeedProvider {
    pub fn new(redis_url: &str, key: &str, channel_type: &str) -> Self {
        Self {
            redis_url: redis_url.to_string(),
            key: if key.is_empty() { "news:incoming".to_string() } else { key.to_string() },
            channel_type: channel_type.to_lowercase(),
            timeout: Duration::from_secs(2),
        }
    }

    fn parse_addr(&self) -> (String, u16) {
        let clean = self.redis_url.trim_start_matches("redis://");
        let host_port = clean.split('/').next().unwrap_or(clean);
        if let Some((h, p)) = host_port.split_once(':') {
            let port = p.parse().unwrap_or(6379);
            (if h.is_empty() { "127.0.0.1".to_string() } else { h.to_string() }, port)
        } else {
            (if host_port.is_empty() { "127.0.0.1".to_string() } else { host_port.to_string() }, 6379)
        }
    }

    pub fn fetch_items(&self, limit: usize) -> Vec<RawNewsItem> {
        let (host, port) = self.parse_addr();
        let addrs = format!("{host}:{port}");
        let socket_addrs = match addrs.to_socket_addrs() {
            Ok(iter) => iter.collect::<Vec<_>>(),
            Err(_) => return Vec::new(),
        };

        let stream = match TcpStream::connect_timeout(
            socket_addrs.first().unwrap_or(&"127.0.0.1:6379".parse().unwrap()),
            self.timeout,
        ) {
            Ok(s) => s,
            Err(_) => return Vec::new(),
        };

        let _ = stream.set_read_timeout(Some(self.timeout));
        let _ = stream.set_write_timeout(Some(self.timeout));

        if self.channel_type == "list" {
            self.fetch_list(stream, limit)
        } else {
            self.fetch_stream(stream, limit)
        }
    }

    fn fetch_list(&self, mut stream: TcpStream, limit: usize) -> Vec<RawNewsItem> {
        let stop = if limit > 0 { limit - 1 } else { 0 };
        let cmd = format!(
            "*4\r\n$6\r\nLRANGE\r\n${}\r\n{}\r\n$1\r\n0\r\n${}\r\n{}\r\n",
            self.key.len(),
            self.key,
            stop.to_string().len(),
            stop
        );
        if stream.write_all(cmd.as_bytes()).is_err() {
            return Vec::new();
        }

        let mut reader = BufReader::new(stream);
        let entries = read_resp_array_strings(&mut reader).unwrap_or_default();
        let mut items = Vec::new();
        for (i, entry) in entries.into_iter().enumerate() {
            if let Some(item) = parse_raw_entry(&entry, &format!("idx_{i}"), &self.key, "list") {
                items.push(item);
            }
        }
        items
    }

    fn fetch_stream(&self, mut stream: TcpStream, limit: usize) -> Vec<RawNewsItem> {
        let count_str = limit.to_string();
        let cmd = format!(
            "*6\r\n$9\r\nXREVRANGE\r\n${}\r\n{}\r\n$1\r\n+\r\n$1\r\n-\r\n$5\r\nCOUNT\r\n${}\r\n{}\r\n",
            self.key.len(),
            self.key,
            count_str.len(),
            count_str
        );
        if stream.write_all(cmd.as_bytes()).is_err() {
            return Vec::new();
        }

        let mut reader = BufReader::new(stream);
        let entries = read_resp_stream_entries(&mut reader).unwrap_or_default();
        let mut items = Vec::new();
        for (msg_id, json_payload) in entries {
            if let Some(item) = parse_raw_entry(&json_payload, &msg_id, &self.key, "stream") {
                items.push(item);
            }
        }
        items
    }
}

pub fn parse_raw_entry(raw: &str, msg_id: &str, key: &str, ch_type: &str) -> Option<RawNewsItem> {
    let parsed: Value = serde_json::from_str(raw).unwrap_or_else(|_| {
        serde_json::json!({
            "title": raw,
            "body": raw
        })
    });

    let obj = parsed.as_object()?;
    let title = obj
        .get("title")
        .and_then(|v| v.as_str())
        .unwrap_or(raw)
        .trim()
        .to_string();
    if title.is_empty() {
        return None;
    }

    let url = obj
        .get("url")
        .or_else(|| obj.get("link"))
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .trim()
        .to_string();
    let effective_url = if url.is_empty() {
        format!("redis://{key}/{msg_id}")
    } else {
        url
    };

    let pub_at = obj
        .get("published_at")
        .or_else(|| obj.get("published_at_utc"))
        .or_else(|| obj.get("created_at"))
        .and_then(|v| v.as_str())
        .map(str::to_string)
        .unwrap_or_else(|| {
            if let Some((ts_ms_str, _)) = msg_id.split_once('-') {
                if let Ok(ts_ms) = ts_ms_str.parse::<u64>() {
                    return format_utc_timestamp(ts_ms / 1000);
                }
            }
            "2026-11-02T11:00:00+00:00".to_string()
        });

    let body = obj
        .get("body")
        .or_else(|| obj.get("description"))
        .or_else(|| obj.get("summary"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let disc_url = obj
        .get("discussion_url")
        .or_else(|| obj.get("comments_url"))
        .and_then(|v| v.as_str())
        .map(str::to_string);

    let mut extra = HashMap::new();
    extra.insert("channel_type".to_string(), Value::String(ch_type.to_string()));
    extra.insert("redis_key".to_string(), Value::String(key.to_string()));
    extra.insert("msg_id".to_string(), Value::String(msg_id.to_string()));

    Some(RawNewsItem {
        id: format!("redis_{msg_id}"),
        url: effective_url,
        title,
        published_at: pub_at,
        publisher: obj.get("publisher").and_then(|v| v.as_str()).map(str::to_string),
        author: obj.get("author").and_then(|v| v.as_str()).map(str::to_string),
        description: body.clone(),
        body,
        discussion_url: disc_url,
        tags: vec!["redis".to_string(), ch_type.to_string()],
        is_evergreen: false,
        content_kind: Some("news".to_string()),
        updated_at: None,
        hook: obj.get("hook").and_then(|v| v.as_str()).map(str::to_string),
        viewer_question: obj.get("viewer_question").and_then(|v| v.as_str()).map(str::to_string),
        symptom: obj.get("symptom").and_then(|v| v.as_str()).map(str::to_string),
        root_cause: obj.get("root_cause").and_then(|v| v.as_str()).map(str::to_string),
        remedy: obj.get("remedy").and_then(|v| v.as_str()).map(str::to_string),
        evidence_excerpt: obj.get("evidence_excerpt").and_then(|v| v.as_str()).map(str::to_string),
        source_snapshot: obj.get("source_snapshot").and_then(|v| v.as_str()).map(str::to_string),
        snapshot_sha256: obj.get("snapshot_sha256").and_then(|v| v.as_str()).map(str::to_string),
        extra,
    })
}

fn read_line<R: BufRead>(reader: &mut R) -> Option<String> {
    let mut line = String::new();
    if reader.read_line(&mut line).ok()? == 0 {
        return None;
    }
    Some(line.trim_end_matches("\r\n").trim_end_matches('\n').to_string())
}

fn read_resp_bulk_string<R: BufRead>(reader: &mut R) -> Option<String> {
    let line = read_line(reader)?;
    if !line.starts_with('$') {
        return None;
    }
    let len: i64 = line[1..].parse().ok()?;
    if len < 0 {
        return None;
    }
    let len = len as usize;
    let mut buf = vec![0u8; len];
    std::io::Read::read_exact(reader, &mut buf).ok()?;
    let mut crlf = [0u8; 2];
    std::io::Read::read_exact(reader, &mut crlf).ok()?;
    String::from_utf8(buf).ok()
}

fn read_resp_array_strings<R: BufRead>(reader: &mut R) -> Option<Vec<String>> {
    let line = read_line(reader)?;
    if !line.starts_with('*') {
        return None;
    }
    let count: usize = line[1..].parse().ok()?;
    let mut result = Vec::with_capacity(count);
    for _ in 0..count {
        if let Some(s) = read_resp_bulk_string(reader) {
            result.push(s);
        }
    }
    Some(result)
}

fn read_resp_stream_entries<R: BufRead>(reader: &mut R) -> Option<Vec<(String, String)>> {
    let line = read_line(reader)?;
    if !line.starts_with('*') {
        return None;
    }
    let count: usize = line[1..].parse().ok()?;
    let mut entries = Vec::with_capacity(count);
    for _ in 0..count {
        let entry_line = read_line(reader)?;
        if !entry_line.starts_with("*2") {
            continue;
        }
        let msg_id = read_resp_bulk_string(reader).unwrap_or_default();
        let fields_line = read_line(reader)?;
        let num_fields: usize = if fields_line.starts_with('*') {
            fields_line[1..].parse().unwrap_or(0)
        } else {
            0
        };
        let mut map = HashMap::new();
        for _ in 0..(num_fields / 2) {
            let k = read_resp_bulk_string(reader).unwrap_or_default();
            let v = read_resp_bulk_string(reader).unwrap_or_default();
            map.insert(k, v);
        }
        let json_str = serde_json::to_string(&map).unwrap_or_default();
        entries.push((msg_id, json_str));
    }
    Some(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_redis_provider_address_parsing() {
        let p1 = RedisFeedProvider::new("redis://localhost:6379/0", "news:stream", "stream");
        assert_eq!(p1.parse_addr(), ("localhost".to_string(), 6379));

        let p2 = RedisFeedProvider::new("redis://127.0.0.1:6380", "news:queue", "list");
        assert_eq!(p2.parse_addr(), ("127.0.0.1".to_string(), 6380));
    }

    #[test]
    fn test_parse_raw_entry() {
        let raw = r#"{"title": "Agent Benchmark Matrix", "url": "https://example.com/matrix", "publisher": "AgentBench"}"#;
        let item = parse_raw_entry(raw, "msg-123", "news:queue", "list").unwrap();
        assert_eq!(item.title, "Agent Benchmark Matrix");
        assert_eq!(item.url, "https://example.com/matrix");
        assert_eq!(item.publisher.as_deref(), Some("AgentBench"));
    }

    #[test]
    fn test_offline_redis_resilience() {
        let provider = RedisFeedProvider::new("redis://127.0.0.1:65534", "dummy:key", "stream");
        let items = provider.fetch_items(5);
        assert!(items.is_empty());
    }
}
