//! Shared reservation domain logic and helpers for distributed handoff coordination.

use crate::domain::{HandoffReservation, ReservationStatus};

pub fn current_epoch_seconds() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(1793618000)
}

pub fn check_reservation_claimable(
    existing: Option<&HandoffReservation>,
    candidate_id: &str,
    is_consumed: bool,
    now: u64,
) -> Result<(), String> {
    if let Some(res) = existing {
        if res.status == ReservationStatus::Published {
            return Err(format!("Candidate '{candidate_id}' is already PUBLISHED"));
        }
        if res.status == ReservationStatus::Reserved && now < res.expires_at {
            return Err(format!(
                "Candidate '{candidate_id}' is actively reserved by worker '{}' until {}",
                res.worker_id, res.expires_at
            ));
        }
    }

    if is_consumed {
        return Err(format!("Candidate '{candidate_id}' is already consumed"));
    }

    Ok(())
}

pub fn create_handoff_reservation(
    candidate_id: &str,
    worker_id: &str,
    lease_seconds: u64,
    now: u64,
) -> (String, HandoffReservation) {
    let token = format!("claim_{}_{}", uuid::Uuid::new_v4(), candidate_id);
    let token_hash = crate::url::compute_sha256(&token);
    let reservation = HandoffReservation {
        reservation_id: format!("res_{}", uuid::Uuid::new_v4()),
        candidate_id: candidate_id.to_string(),
        token_hash,
        worker_id: worker_id.to_string(),
        created_at: crate::date::current_utc_timestamp(),
        expires_at: now + lease_seconds,
        status: ReservationStatus::Reserved,
    };
    (token, reservation)
}

pub fn validate_reservation_commit(
    res: &HandoffReservation,
    candidate_id: &str,
    token: &str,
    now: u64,
) -> Result<bool, String> {
    let token_hash = crate::url::compute_sha256(token);
    if res.token_hash != token_hash {
        return Err(format!(
            "Invalid reservation token for candidate '{candidate_id}'"
        ));
    }

    if res.status == ReservationStatus::Published {
        return Ok(true);
    }

    if now > res.expires_at {
        return Err(format!(
            "Reservation lease for candidate '{candidate_id}' has expired"
        ));
    }

    Ok(false)
}
