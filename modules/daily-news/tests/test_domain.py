"""Unit tests for daily_news domain models and pure functions."""

import unittest

from daily_news.domain import (
    canonicalize_url,
    compute_content_hash,
    compute_title_hash,
    is_safe_url,
    normalize_publisher,
    parse_iso_utc,
)


class TestDomainPureFunctions(unittest.TestCase):
    def test_canonicalize_url(self):
        url = "https://example.com/posts/1/?utm_source=twitter&utm_medium=social&fbclid=xyz#section"
        canonical = canonicalize_url(url)
        self.assertEqual(canonical, "https://example.com/posts/1")

        # Test lowercase host and scheme
        self.assertEqual(canonicalize_url("HTTP://EXAMPLE.COM/path/"), "http://example.com/path")

    def test_normalize_publisher(self):
        self.assertEqual(normalize_publisher("The Verge.com"), "The Verge")
        self.assertEqual(normalize_publisher("  TechCrunch.io  "), "TechCrunch")
        self.assertEqual(normalize_publisher(None), "Unknown")
        self.assertEqual(normalize_publisher(""), "Unknown")

    def test_parse_iso_utc(self):
        iso1 = parse_iso_utc("2026-11-01T12:00:00Z")
        self.assertTrue(iso1.endswith("+00:00") or iso1.endswith("Z"))

        iso2 = parse_iso_utc("2026-11-01 12:00:00")
        self.assertTrue(iso2.startswith("2026-11-01T12:00:00"))

        with self.assertRaises(ValueError):
            parse_iso_utc("invalid-timestamp")

    def test_compute_hashes(self):
        t1 = "OpenAI Announces New Agent SDK"
        t2 = "openai  announces new agent sdk "
        self.assertEqual(compute_title_hash(t1), compute_title_hash(t2))

        c1 = compute_content_hash(t1, "Body text here", "https://example.com/sdk")
        c2 = compute_content_hash(t2, "Body text here", "https://example.com/sdk")
        self.assertEqual(c1, c2)

    def test_is_safe_url(self):
        allowed = {"github.com", "anthropic.com"}
        self.assertTrue(is_safe_url("https://github.com/torvalds/linux", allowed))
        self.assertTrue(is_safe_url("https://docs.anthropic.com/en/docs", allowed))
        self.assertFalse(is_safe_url("https://malicious-site.com/hack", allowed))
        self.assertFalse(is_safe_url("http://192.168.1.1/secret", allowed))
        self.assertFalse(is_safe_url("http://10.0.0.1/private", allowed))


if __name__ == "__main__":
    unittest.main()
