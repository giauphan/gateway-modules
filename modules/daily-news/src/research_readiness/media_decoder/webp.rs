//! RFC 9649 WebP image stream decoding and subchunk validation.

pub fn validate_webp_vp8_payload(payload: &[u8]) -> bool {
    if payload.len() < 10 {
        return false;
    }
    // VP8 keyframe start code 0x9D 0x01 0x2A at payload bytes 3..6
    let has_keyframe_code = payload[3..6] == [0x9D, 0x01, 0x2A];
    let width = (u16::from_le_bytes([payload[6], payload[7]]) & 0x3FFF) as u32;
    let height = (u16::from_le_bytes([payload[8], payload[9]]) & 0x3FFF) as u32;
    has_keyframe_code && width > 0 && height > 0
}

pub fn validate_webp_vp8l_payload(payload: &[u8]) -> bool {
    if payload.len() < 5 || payload[0] != 0x2F {
        return false;
    }
    let b1 = payload[1] as u32;
    let b2 = payload[2] as u32;
    let b3 = payload[3] as u32;
    let b4 = payload[4] as u32;
    let width = 1 + (((b2 & 0x3F) << 8) | b1);
    let height = 1 + (((b4 & 0x0F) << 10) | (b3 << 2) | ((b2 & 0xC0) >> 6));
    width > 0 && height > 0
}

pub fn has_webp_sub_chunk(bytes: &[u8], mut sub_pos: usize) -> bool {
    while sub_pos + 8 <= bytes.len() {
        let sub_tag = &bytes[sub_pos..sub_pos + 4];
        let sub_size = u32::from_le_bytes([
            bytes[sub_pos + 4],
            bytes[sub_pos + 5],
            bytes[sub_pos + 6],
            bytes[sub_pos + 7],
        ]) as usize;
        let sub_data_start = sub_pos + 8;
        let Some(sub_data_end) = sub_data_start.checked_add(sub_size) else {
            return false;
        };
        // Verify subchunk payload is strictly bounded per RFC 9649
        if sub_data_end > bytes.len() {
            return false;
        }
        if sub_tag == b"VP8 " {
            return validate_webp_vp8_payload(&bytes[sub_data_start..sub_data_end]);
        }
        if sub_tag == b"VP8L" {
            return validate_webp_vp8l_payload(&bytes[sub_data_start..sub_data_end]);
        }
        if sub_tag == b"ANIM" {
            return sub_size >= 6;
        }
        sub_pos = sub_data_end;
        if sub_size % 2 == 1 {
            sub_pos += 1;
        }
    }
    false
}

pub fn validate_webp_vp8x(bytes: &[u8], chunk_size: usize) -> bool {
    if bytes.len() < 30 || chunk_size < 10 {
        return false;
    }
    let width = 1 + (bytes[24] as u32 | ((bytes[25] as u32) << 8) | ((bytes[26] as u32) << 16));
    let height = 1 + (bytes[27] as u32 | ((bytes[28] as u32) << 8) | ((bytes[29] as u32) << 16));
    if width == 0 || height == 0 {
        return false;
    }
    let sub_pos = 20 + chunk_size + (chunk_size % 2);
    has_webp_sub_chunk(bytes, sub_pos)
}

pub fn decode_webp_stream(bytes: &[u8]) -> bool {
    if bytes.len() < 20 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WEBP" {
        return false;
    }
    let riff_size = u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
    if riff_size < 12 || bytes.len() + 1 < riff_size + 8 {
        return false;
    }
    let chunk_tag = &bytes[12..16];
    let chunk_size = u32::from_le_bytes([bytes[16], bytes[17], bytes[18], bytes[19]]) as usize;
    if 20 + chunk_size > bytes.len() {
        return false;
    }
    match chunk_tag {
        b"VP8 " => validate_webp_vp8_payload(&bytes[20..20 + chunk_size]),
        b"VP8L" => validate_webp_vp8l_payload(&bytes[20..20 + chunk_size]),
        b"VP8X" => validate_webp_vp8x(bytes, chunk_size),
        _ => false,
    }
}
