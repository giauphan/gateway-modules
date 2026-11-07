//! ISO-BMFF (ISO/IEC 14496-12) container and video track parser.
//!
//! Enforces strict structural validation of MP4 visual media assets:
//! - Valid `ftyp` brand container
//! - Structural `moov` -> `trak` -> `mdia` box hierarchy
//! - `hdlr` handler reference of type `vide`
//! - `minf` containing `vmhd` (video media header) and `stbl` (sample table)
//! - `stsd` visual sample entry with non-zero dimensions
//! - `stsz` / `stz2` sample size table confirming positive sample / frame count (`sample_count > 0`)
//! - `stco` / `co64` chunk offset table pointing within `mdat` payload boundaries

pub fn parse_mp4_box_length(bytes: &[u8], pos: usize, box_len: usize) -> Option<usize> {
    if box_len == 0 {
        Some(bytes.len() - pos)
    } else if box_len == 1 {
        if pos + 16 > bytes.len() {
            return None;
        }
        let l64 = u64::from_be_bytes([
            bytes[pos + 8], bytes[pos + 9], bytes[pos + 10], bytes[pos + 11],
            bytes[pos + 12], bytes[pos + 13], bytes[pos + 14], bytes[pos + 15],
        ]);
        let l = usize::try_from(l64).ok()?;
        if l < 16 {
            return None;
        }
        Some(l)
    } else if box_len < 8 {
        None
    } else {
        Some(box_len)
    }
}

pub fn iter_boxes<F>(bytes: &[u8], mut start: usize, end: usize, mut callback: F) -> bool
where
    F: FnMut(&[u8; 4], usize, usize) -> bool,
{
    while start + 8 <= end {
        let box_len = u32::from_be_bytes([bytes[start], bytes[start + 1], bytes[start + 2], bytes[start + 3]]) as usize;
        let mut box_type = [0u8; 4];
        box_type.copy_from_slice(&bytes[start + 4..start + 8]);
        let Some(effective_len) = parse_mp4_box_length(bytes, start, box_len) else {
            return false;
        };
        let Some(next_start) = start.checked_add(effective_len) else {
            return false;
        };
        if next_start > end || next_start <= start {
            return false;
        }
        let header_len = if box_len == 1 { 16 } else { 8 };
        if !callback(&box_type, start + header_len, next_start) {
            return false;
        }
        start = next_start;
    }
    true
}

fn validate_sample_description_box(bytes: &[u8], start: usize, end: usize) -> bool {
    // 1 byte version, 3 bytes flags, 4 bytes entry_count
    if start + 8 > end {
        return false;
    }
    let entry_count = u32::from_be_bytes([bytes[start + 4], bytes[start + 5], bytes[start + 6], bytes[start + 7]]);
    if entry_count == 0 {
        return false;
    }

    // Inspect first sample entry: 4 bytes len, 4 bytes format (e.g. avc1, mp4v, hvc1, vp09, av01)
    let entry_start = start + 8;
    if entry_start + 8 > end {
        return false;
    }
    let entry_len = u32::from_be_bytes([
        bytes[entry_start], bytes[entry_start + 1], bytes[entry_start + 2], bytes[entry_start + 3]
    ]) as usize;
    if entry_len < 8 || entry_start + entry_len > end {
        return false;
    }

    // VisualSampleEntry specifies width at byte 24 and height at byte 26
    if entry_len >= 28 {
        let width = u16::from_be_bytes([bytes[entry_start + 24], bytes[entry_start + 25]]);
        let height = u16::from_be_bytes([bytes[entry_start + 26], bytes[entry_start + 27]]);
        return width > 0 && height > 0;
    }

    true
}

fn validate_sample_size_box(bytes: &[u8], start: usize, end: usize, is_stz2: bool) -> bool {
    if !is_stz2 {
        // stsz: 4 bytes flags/version, 4 bytes sample_size, 4 bytes sample_count
        if start + 12 > end {
            return false;
        }
        let sample_size = u32::from_be_bytes([bytes[start + 4], bytes[start + 5], bytes[start + 6], bytes[start + 7]]);
        let sample_count = u32::from_be_bytes([bytes[start + 8], bytes[start + 9], bytes[start + 10], bytes[start + 11]]);
        if sample_count == 0 {
            return false;
        }
        if sample_size == 0 {
            let needed = (sample_count as usize).checked_mul(4);
            if needed.is_none_or(|n| start + 12 + n > end) {
                return false;
            }
        }
        true
    } else {
        // stz2: 4 bytes flags/version, 3 reserved, 1 field_size, 4 sample_count
        if start + 12 > end {
            return false;
        }
        let sample_count = u32::from_be_bytes([bytes[start + 8], bytes[start + 9], bytes[start + 10], bytes[start + 11]]);
        sample_count > 0
    }
}

fn validate_chunk_offset_box(bytes: &[u8], start: usize, end: usize, is_co64: bool, mdat_range: (usize, usize)) -> bool {
    // 4 bytes flags/version, 4 bytes entry_count
    if start + 8 > end {
        return false;
    }
    let entry_count = u32::from_be_bytes([bytes[start + 4], bytes[start + 5], bytes[start + 6], bytes[start + 7]]) as usize;
    if entry_count == 0 {
        return false;
    }

    let (mdat_start, mdat_end) = mdat_range;
    if !is_co64 {
        if start + 8 + 4 > end {
            return false;
        }
        let first_offset = u32::from_be_bytes([
            bytes[start + 8], bytes[start + 9], bytes[start + 10], bytes[start + 11]
        ]) as usize;
        first_offset >= mdat_start && first_offset < mdat_end
    } else {
        if start + 8 + 8 > end {
            return false;
        }
        let first_offset = u64::from_be_bytes([
            bytes[start + 8], bytes[start + 9], bytes[start + 10], bytes[start + 11],
            bytes[start + 12], bytes[start + 13], bytes[start + 14], bytes[start + 15],
        ]) as usize;
        first_offset >= mdat_start && first_offset < mdat_end
    }
}

fn validate_stbl(bytes: &[u8], start: usize, end: usize, mdat_range: (usize, usize)) -> bool {
    let mut saw_stsd = false;
    let mut saw_positive_samples = false;
    let mut saw_valid_chunk_offset = false;

    let ok = iter_boxes(bytes, start, end, |box_type, c_start, c_end| {
        match box_type {
            b"stsd" => {
                if validate_sample_description_box(bytes, c_start, c_end) {
                    saw_stsd = true;
                }
            }
            b"stsz" => {
                if validate_sample_size_box(bytes, c_start, c_end, false) {
                    saw_positive_samples = true;
                }
            }
            b"stz2" => {
                if validate_sample_size_box(bytes, c_start, c_end, true) {
                    saw_positive_samples = true;
                }
            }
            b"stco" if validate_chunk_offset_box(bytes, c_start, c_end, false, mdat_range) => {
                saw_valid_chunk_offset = true;
            }
            b"co64" if validate_chunk_offset_box(bytes, c_start, c_end, true, mdat_range) => {
                saw_valid_chunk_offset = true;
            }
            _ => {}
        }
        true
    });

    ok && saw_stsd && saw_positive_samples && saw_valid_chunk_offset
}

fn validate_minf(bytes: &[u8], start: usize, end: usize, mdat_range: (usize, usize)) -> bool {
    let mut saw_vmhd = false;
    let mut valid_stbl = false;

    let ok = iter_boxes(bytes, start, end, |box_type, c_start, c_end| {
        match box_type {
            b"vmhd" => {
                // Video media header must be at least 12 bytes (4 flags/version, 2 graphicsmode, 6 opcolor)
                if c_end >= c_start + 12 {
                    saw_vmhd = true;
                }
            }
            b"stbl" if validate_stbl(bytes, c_start, c_end, mdat_range) => {
                valid_stbl = true;
            }
            _ => {}
        }
        true
    });

    ok && saw_vmhd && valid_stbl
}

fn validate_mdia(bytes: &[u8], start: usize, end: usize, mdat_range: (usize, usize)) -> bool {
    let mut saw_vide_hdlr = false;
    let mut valid_minf = false;

    let ok = iter_boxes(bytes, start, end, |box_type, c_start, c_end| {
        match box_type {
            b"hdlr" => {
                // 4 bytes version/flags, 4 bytes pre_defined, 4 bytes handler_type
                if c_start + 12 <= c_end && &bytes[c_start + 8..c_start + 12] == b"vide" {
                    saw_vide_hdlr = true;
                }
            }
            b"minf" if validate_minf(bytes, c_start, c_end, mdat_range) => {
                valid_minf = true;
            }
            _ => {}
        }
        true
    });

    ok && saw_vide_hdlr && valid_minf
}

fn validate_video_trak(bytes: &[u8], start: usize, end: usize, mdat_range: (usize, usize)) -> bool {
    let mut has_valid_video_media = false;
    let ok = iter_boxes(bytes, start, end, |box_type, c_start, c_end| {
        if box_type == b"mdia" && validate_mdia(bytes, c_start, c_end, mdat_range) {
            has_valid_video_media = true;
        }
        true
    });
    ok && has_valid_video_media
}

pub fn decode_mp4_stream(bytes: &[u8]) -> bool {
    if bytes.len() < 32 || &bytes[4..8] != b"ftyp" {
        return false;
    }

    let mut saw_ftyp = false;
    let mut moov_range = None;
    let mut mdat_range = None;

    let ok = iter_boxes(bytes, 0, bytes.len(), |box_type, c_start, c_end| {
        match box_type {
            b"ftyp" => {
                saw_ftyp = true;
            }
            b"moov" => {
                moov_range = Some((c_start, c_end));
            }
            b"mdat" if c_end >= c_start + 16 => {
                // Require non-empty frame media payload (at least 16 bytes)
                mdat_range = Some((c_start, c_end));
            }
            _ => {}
        }
        true
    });

    if !ok || !saw_ftyp {
        return false;
    }

    let (Some(moov), Some(mdat)) = (moov_range, mdat_range) else {
        return false;
    };

    // Parse traks within moov and verify that at least one valid video track exists
    let mut saw_valid_video_track = false;
    let moov_ok = iter_boxes(bytes, moov.0, moov.1, |box_type, c_start, c_end| {
        if box_type == b"trak" && validate_video_trak(bytes, c_start, c_end, mdat) {
            saw_valid_video_track = true;
        }
        true
    });

    moov_ok && saw_valid_video_track
}
