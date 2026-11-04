//! Visual evidence validation, allowed licenses, and anti-traversal safety checks.

use super::types::{AssetResolver, VerifiedAssetRecord, VisualEvidence};

pub const APPROVED_LICENSES: &[&str] = &[
    "cc-by",
    "cc-by-sa",
    "cc0",
    "public-domain",
    "mit",
    "apache-2.0",
    "fair-use",
    "licensed",
    "approved",
    "editorial",
    "authorized",
];

pub fn is_visual_evidence_valid(v: &VisualEvidence) -> bool {
    let asset_ref = v.asset_ref.trim();
    let source_url = v.source_url.trim();

    if asset_ref.is_empty() {
        return false;
    }
    let lower_ref = asset_ref.to_lowercase();
    if matches!(
        lower_ref.as_str(),
        "none" | "null" | "placeholder" | "dummy" | "test" | "untitled"
    ) {
        return false;
    }

    if asset_ref.contains("..")
        || asset_ref.contains('\\')
        || asset_ref.starts_with('/')
        || v.source_snapshot_ref
            .as_deref()
            .unwrap_or("")
            .contains("..")
        || v.source_snapshot_ref
            .as_deref()
            .unwrap_or("")
            .contains('\\')
    {
        return false;
    }

    if lower_ref.starts_with("javascript:")
        || lower_ref.starts_with("data:")
        || lower_ref.starts_with("file:")
    {
        return false;
    }

    let valid_kind = matches!(
        v.kind.trim().to_lowercase().as_str(),
        "chart"
            | "graph"
            | "diagram"
            | "photo"
            | "image"
            | "screenshot"
            | "video"
            | "demo"
            | "figure"
            | "infographic"
    );
    if !valid_kind {
        return false;
    }

    if source_url.is_empty() || !crate::url::is_safe_url(source_url, None) {
        return false;
    }
    let lower_url = source_url.to_lowercase();
    if lower_url.starts_with("javascript:")
        || lower_url.starts_with("data:")
        || lower_url.starts_with("file:")
    {
        return false;
    }

    let valid_prov = v.provenance_status.as_deref().is_some_and(|p| {
        matches!(
            p.trim().to_lowercase().as_str(),
            "verified" | "authoritative" | "licensed" | "approved"
        )
    });

    let approved_lic = v
        .license_status
        .as_deref()
        .is_some_and(|l| APPROVED_LICENSES.contains(&l.trim().to_lowercase().as_str()));

    let valid_checksum = v.checksum_sha256.as_deref().is_some_and(|h| {
        let trimmed = h.trim();
        trimmed.len() == 64 && trimmed.chars().all(|c| c.is_ascii_hexdigit())
    });

    let valid_license_evidence = v.license_evidence_ref.as_deref().is_some_and(|r| {
        let tr = r.trim();
        !tr.is_empty() && !tr.contains("..") && !tr.contains('\\')
    });

    let verified_rights = v.verified_rights.unwrap_or(false);

    valid_prov && approved_lic && valid_checksum && valid_license_evidence && verified_rights
}

pub fn verify_visual_asset_record(
    v: &VisualEvidence,
    asset_bytes: Option<&[u8]>,
    license_doc: Option<&str>,
) -> Result<VerifiedAssetRecord, String> {
    if !is_visual_evidence_valid(v) {
        return Err("Visual evidence metadata validation failed".to_string());
    }

    let expected_sha = v
        .checksum_sha256
        .as_deref()
        .ok_or_else(|| "Missing checksum_sha256 on visual asset".to_string())?
        .trim()
        .to_ascii_lowercase();

    let bytes_verified = match asset_bytes {
        Some(bytes) => {
            if bytes.is_empty() {
                return Err("Visual asset bytes are empty".to_string());
            }
            let computed = crate::url::compute_sha256_bytes(bytes);
            if computed != expected_sha {
                return Err(format!(
                    "Visual asset checksum mismatch: expected {expected_sha}, computed {computed}"
                ));
            }
            true
        }
        None => false,
    };

    let license_verified = match license_doc {
        Some(doc) => {
            let tr = doc.trim();
            if tr.is_empty() {
                return Err("License evidence document is empty".to_string());
            }
            true
        }
        None => v.verified_rights.unwrap_or(false),
    };

    Ok(VerifiedAssetRecord {
        asset_id: v.asset_ref.clone(),
        checksum_sha256: expected_sha,
        bytes_verified,
        license_verified,
        verified_at: "2026-11-04T00:00:00Z".to_string(),
        storage_path: v.source_snapshot_ref.clone(),
    })
}

pub fn are_visuals_provenance_verified(
    visuals: &[VisualEvidence],
    asset_resolver: AssetResolver,
) -> bool {
    if visuals.is_empty() {
        return false;
    }

    visuals.iter().all(|v| {
        if !is_visual_evidence_valid(v) {
            return false;
        }

        if let Some(resolver) = asset_resolver {
            let expected_sha = match v.checksum_sha256.as_deref() {
                Some(sha) => sha.trim().to_ascii_lowercase(),
                None => return false,
            };
            match resolver(&expected_sha) {
                Some(bytes) => {
                    let computed = crate::url::compute_sha256_bytes(&bytes);
                    if computed != expected_sha {
                        return false;
                    }
                }
                None => return false,
            }
        }

        true
    })
}

pub fn are_visuals_verified(visuals: &[VisualEvidence]) -> bool {
    are_visuals_provenance_verified(visuals, None)
}
