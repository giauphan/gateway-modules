"""Unit tests for daily_news domain models and pure functions."""

import unittest

from daily_news.domain import (
    canonicalize_url,
    compute_content_hash,
    compute_title_hash,
    is_safe_url,
    normalize_publisher,
    parse_boundary_datetime,
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

    def test_parse_boundary_datetime(self):
        self.assertIsNone(parse_boundary_datetime(None))
        self.assertIsNone(parse_boundary_datetime(""))
        self.assertIsNone(parse_boundary_datetime("   "))

        # Calendar date start-of-day
        dt_start = parse_boundary_datetime("2026-10-28", is_end_of_day=False)
        self.assertIsNotNone(dt_start)
        self.assertEqual(dt_start.year, 2026)
        self.assertEqual(dt_start.month, 10)
        self.assertEqual(dt_start.day, 28)
        self.assertEqual(dt_start.hour, 0)
        self.assertEqual(dt_start.minute, 0)
        self.assertEqual(dt_start.second, 0)

        # Calendar date end-of-day
        dt_end = parse_boundary_datetime("2026-10-28", is_end_of_day=True)
        self.assertIsNotNone(dt_end)
        self.assertEqual(dt_end.year, 2026)
        self.assertEqual(dt_end.month, 10)
        self.assertEqual(dt_end.day, 28)
        self.assertEqual(dt_end.hour, 23)
        self.assertEqual(dt_end.minute, 59)
        self.assertEqual(dt_end.second, 59)

        # Full ISO-8601
        dt_iso = parse_boundary_datetime("2026-11-01T15:30:00Z")
        self.assertEqual(dt_iso.hour, 15)
        self.assertEqual(dt_iso.minute, 30)

        with self.assertRaises(ValueError):
            parse_boundary_datetime("not-a-valid-date")

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


class TestDomainModels(unittest.TestCase):
    def test_raw_news_item_discussion_url(self):
        from daily_news.domain import RawNewsItem, NormalizedCandidate

        raw = RawNewsItem(
            id="test_1",
            url="https://wikihack.netlify.app",
            title="Show HN: Wikihack",
            publisher="Hacker News",
            published_at="2026-11-02T10:00:00Z",
            discussion_url="https://news.ycombinator.com/item?id=50333339",
        )
        self.assertEqual(raw.discussion_url, "https://news.ycombinator.com/item?id=50333339")

        # from_dict with direct key
        d1 = {"id": "1", "url": "https://example.com", "title": "Ex", "discussion_url": "https://hn.com/1"}
        item1 = RawNewsItem.from_dict(d1)
        self.assertEqual(item1.discussion_url, "https://hn.com/1")

        # from_dict with extra key
        d2 = {"id": "2", "url": "https://example.com", "title": "Ex", "extra": {"discussion_url": "https://hn.com/2"}}
        item2 = RawNewsItem.from_dict(d2)
        self.assertEqual(item2.discussion_url, "https://hn.com/2")

        cand = NormalizedCandidate(
            candidate_id="cand_1",
            original_url="https://wikihack.netlify.app",
            canonical_url="https://wikihack.netlify.app",
            title="Show HN: Wikihack",
            publisher="Hacker News",
            normalized_publisher="Hacker News",
            published_at_utc="2026-11-02T10:00:00+00:00",
            content_hash="chash123",
            title_hash="thash123",
            raw_provenance={"id": "test_1"},
            discussion_url="https://news.ycombinator.com/item?id=50333339",
        )
        cand_dict = cand.to_dict()
        self.assertEqual(cand_dict["canonical_url"], "https://wikihack.netlify.app")
        self.assertEqual(cand_dict["discussion_url"], "https://news.ycombinator.com/item?id=50333339")


if __name__ == "__main__":
    unittest.main()
