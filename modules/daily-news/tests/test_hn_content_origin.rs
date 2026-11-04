use daily_news::domain::{ContentOrigin, NormalizedCandidate};
use daily_news::hn::parse_hn_item_json;
use daily_news::scoring::CandidateScorer;
use daily_news::url::classify_topic_family;
use serde_json::json;

#[test]
fn test_hn_external_link_does_not_use_comments_as_body() {
    let payload = json!({
        "id": 50351111,
        "type": "story",
        "by": "phoghed",
        "time": 1730636477,
        "title": "The 'Astra can read and write weights to improve Qwen' post is misleading",
        "url": "https://honnibal.dev/blog/astra-read-write-misleading",
        "kids": [50351200, 50351300],
        "score": 42,
        "descendants": 12
    });

    let top_comments = vec![
        "alice: This blog post raises critical architectural points.".to_string(),
        "bob: The benchmark claims could not be reproduced.".to_string(),
    ];

    let item = parse_hn_item_json(&payload, 50351111, &top_comments).expect("Should parse item");

    // P0 Requirement: When HN story lacks selftext, comments must NEVER become body or description
    assert_ne!(
        item.body.as_deref(),
        Some("alice: This blog post raises critical architectural points. | bob: The benchmark claims could not be reproduced."),
        "HN comments must NOT become the article body"
    );
    assert_ne!(
        item.description.as_deref(),
        Some("alice: This blog post raises critical architectural points. | bob: The benchmark claims could not be reproduced."),
        "HN comments must NOT become the article description"
    );

    // Body for external link without fetched article must be None or empty
    assert!(item.body.is_none() || item.body.as_deref().unwrap().is_empty());

    // Evidence excerpt must NOT come from comments
    if let Some(excerpt) = item.evidence.evidence_excerpt.as_deref() {
        assert!(
            !excerpt.contains("critical architectural points"),
            "Evidence excerpt must not be extracted from community comments"
        );
    }

    // Provenance must indicate headline_only or needs_source
    assert_eq!(item.content_origin, ContentOrigin::HeadlineOnly);

    // Discussion comments must be preserved in structured discussion field
    assert!(item.discussion.is_some());
    let disc = item.discussion.unwrap();
    assert_eq!(disc.role, "community_opinion");
    assert_eq!(disc.comments.len(), 2);
    assert_eq!(item.submission_author.as_deref(), Some("phoghed"));
}

#[test]
fn test_hn_missing_time_is_not_synthesized_as_now() {
    let payload = json!({
        "id": 50351999,
        "type": "story",
        "by": "undated_author",
        "title": "Undated Story",
        "url": "https://example.com/undated",
        "score": 10
    });

    let item = parse_hn_item_json(&payload, 50351999, &[]).expect("Should parse item");
    assert_ne!(
        item.published_at_kind.as_str(),
        "article_publication",
        "Missing timestamp should not be attributed as confirmed article publication"
    );
    assert_eq!(item.published_at_kind.as_str(), "unknown");
}

#[test]
fn test_visualizability_score_no_discussion_url_bonus() {
    let scorer = CandidateScorer::default();
    let score_without_image_or_discussion =
        scorer.score_visualizability("Refactoring Memory in Rust", "Textual description", false);

    let cand_with_disc_only = NormalizedCandidate {
        candidate_id: "c1".to_string(),
        canonical_url: "https://example.com/item".to_string(),
        original_url: "https://example.com/item".to_string(),
        url: "https://example.com/item".to_string(),
        source_url: "https://example.com/item".to_string(),
        source_type: "news".to_string(),
        title: "Refactoring Memory in Rust".to_string(),
        publisher: "example.com".to_string(),
        normalized_publisher: "example.com".to_string(),
        published_at_utc: "2026-11-03T10:00:00+00:00".to_string(),
        published_at: "2026-11-03T10:00:00+00:00".to_string(),
        content_hash: "hash".to_string(),
        title_hash: "thash".to_string(),
        raw_provenance: Default::default(),
        adapter_version: env!("CARGO_PKG_VERSION").to_string(),
        topic_family: "unclassified".to_string(),
        author: None,
        description: Some("Textual description".to_string()),
        body: Some("Textual description".to_string()),
        summary: None,
        tags: vec![],
        language: "en".to_string(),
        image_url: None, // No image!
        is_evergreen: false,
        content_kind: "news".to_string(),
        updated_at: None,
        discussion_url: Some("https://news.ycombinator.com/item?id=123".to_string()), // Discussion only!
        evidence: Default::default(),
        independent_fact_check: None,
        scoring: None,
        content_origin: ContentOrigin::HeadlineOnly,
        published_at_kind: "hn_submission".to_string(),
        submission_author: None,
        article_author: None,
        hook_origin: None,
        discussion: None,
        primary_source: None,
        claims: vec![],
        visual_evidence: vec![],
        research_readiness: None,
    };

    let scored = scorer.score_candidate(&cand_with_disc_only);
    // Merely having discussion_url must NOT give the visual bonus (which was 0.3)
    assert!(
        (scored.axes.visual_potential - score_without_image_or_discussion).abs() < 0.05,
        "Discussion URL alone must not boost visual potential score"
    );
}

#[test]
fn test_topic_classification_ci_substring_and_unclassified() {
    // "ci" substring inside unrelated word must NOT match debugging_reliability
    let topic_unrelated_ci = classify_topic_family(
        "Specializing Municipal Facilities in Urban Areas",
        "Participating in local civic activities",
    );
    assert_ne!(
        topic_unrelated_ci, "debugging_reliability",
        "'ci' substring within words must not trigger debugging_reliability"
    );

    // Truly off-topic headline must return "unclassified", NOT "agent_architecture"
    let topic_bakery = classify_topic_family(
        "Local Bakery Wins French Pastry Championship",
        "Fresh croissants and baguettes every morning",
    );
    assert_eq!(
        topic_bakery, "unclassified",
        "Off-topic headline must fall back to 'unclassified'"
    );

    // Legitimate CI token should still match
    let topic_real_ci = classify_topic_family(
        "Fixing flaky CI pipeline and retry loops",
        "Automated continuous integration failures",
    );
    assert_eq!(topic_real_ci, "debugging_reliability");
}

#[test]
fn test_feasibility_not_hardcoded_1_for_unverified() {
    let scorer = CandidateScorer::default();
    let unverified_cand = NormalizedCandidate {
        candidate_id: "c2".to_string(),
        canonical_url: "https://example.com/unverified".to_string(),
        original_url: "https://example.com/unverified".to_string(),
        url: "https://example.com/unverified".to_string(),
        source_url: "https://example.com/unverified".to_string(),
        source_type: "news".to_string(),
        title: "Unverified Rumor on AI Breakthrough".to_string(),
        publisher: "unknown.blog".to_string(),
        normalized_publisher: "unknown.blog".to_string(),
        published_at_utc: "2026-11-03T10:00:00+00:00".to_string(),
        published_at: "2026-11-03T10:00:00+00:00".to_string(),
        content_hash: "hash2".to_string(),
        title_hash: "thash2".to_string(),
        raw_provenance: Default::default(),
        adapter_version: env!("CARGO_PKG_VERSION").to_string(),
        topic_family: "unclassified".to_string(),
        author: None,
        description: None,
        body: None,
        summary: None,
        tags: vec![],
        language: "en".to_string(),
        image_url: None,
        is_evergreen: false,
        content_kind: "news".to_string(),
        updated_at: None,
        discussion_url: None,
        evidence: Default::default(),
        independent_fact_check: None,
        scoring: None,
        content_origin: ContentOrigin::HeadlineOnly,
        published_at_kind: "hn_submission".to_string(),
        submission_author: None,
        article_author: None,
        hook_origin: None,
        discussion: None,
        primary_source: None,
        claims: vec![],
        visual_evidence: vec![],
        research_readiness: None,
    };

    let scored = scorer.score_candidate(&unverified_cand);
    assert!(
        scored.axes.feasibility < 0.9,
        "Unverified candidate without primary source must not have feasibility = 1.0"
    );
}
