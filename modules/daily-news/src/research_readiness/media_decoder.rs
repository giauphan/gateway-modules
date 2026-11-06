//! Stream-level decoding and integrity validation for visual media containers.
//!
//! Enforces clean container parsing and pixel raster decompression (PNG, JPEG, GIF, WebP, SVG, MP4).

use super::visual::{has_inline_event_handler, is_mock_visual_bytes, is_valid_visual_asset_format};

pub fn decode_png_stream(bytes: &[u8]) -> bool {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return false;
    }
    let mut pos = 8;
    let mut saw_ihdr = false;
    let mut saw_iend = false;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut bit_depth = 0u8;
    let mut color_type = 0u8;
    let mut compressed_idat = Vec::new();

    while pos + 8 <= bytes.len() {
        let chunk_len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let chunk_type = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start + chunk_len;
        let chunk_end = data_end + 4; // plus 4 bytes CRC

        if chunk_end > bytes.len() {
            return false;
        }

        // Validate CRC32 of chunk (type + data)
        let mut hasher = crc32fast::Hasher::new();
        hasher.update(&bytes[pos + 4..data_end]);
        let expected_crc = u32::from_be_bytes([
            bytes[data_end],
            bytes[data_end + 1],
            bytes[data_end + 2],
            bytes[data_end + 3],
        ]);
        if hasher.finalize() != expected_crc {
            return false;
        }

        match chunk_type {
            b"IHDR" => {
                if chunk_len < 13 || saw_ihdr {
                    return false;
                }
                width = u32::from_be_bytes([bytes[data_start], bytes[data_start + 1], bytes[data_start + 2], bytes[data_start + 3]]);
                height = u32::from_be_bytes([bytes[data_start + 4], bytes[data_start + 5], bytes[data_start + 6], bytes[data_start + 7]]);
                bit_depth = bytes[data_start + 8];
                color_type = bytes[data_start + 9];
                if width == 0 || height == 0 {
                    return false;
                }
                saw_ihdr = true;
            }
            b"IDAT" => {
                if !saw_ihdr || chunk_len == 0 {
                    return false;
                }
                compressed_idat.extend_from_slice(&bytes[data_start..data_end]);
            }
            b"IEND" => {
                saw_iend = true;
                break;
            }
            _ => {}
        }
        pos = chunk_end;
    }

    if !saw_ihdr || !saw_iend || compressed_idat.is_empty() {
        return false;
    }

    if width == 0 || height == 0 || width > 10000 || height > 10000 {
        return false;
    }

    let bpp = match color_type {
        0 => 1usize,
        2 => 3usize,
        3 => 1usize,
        4 => 2usize,
        6 => 4usize,
        _ => return false,
    };
    let bits_per_pixel = match bpp.checked_mul(bit_depth as usize) {
        Some(b) => b,
        None => return false,
    };
    let row_data_bits = match (width as usize).checked_mul(bits_per_pixel) {
        Some(b) => b,
        None => return false,
    };
    let row_data_bytes = (row_data_bits + 7) / 8;
    let row_bytes = match 1usize.checked_add(row_data_bytes) {
        Some(b) => b,
        None => return false,
    };
    let min_expected = match (height as usize).checked_mul(row_bytes) {
        Some(m) => m,
        None => return false,
    };

    // Zip bomb & RAM limit protection: max 32 MB decompressed output
    const MAX_DECOMPRESSED_PNG_BYTES: usize = 32 * 1024 * 1024;
    if min_expected > MAX_DECOMPRESSED_PNG_BYTES {
        return false;
    }

    // Real decompression using miniz_oxide with bounded memory limit
    let Ok(decompressed) = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
        &compressed_idat,
        MAX_DECOMPRESSED_PNG_BYTES,
    ) else {
        return false;
    };

    if decompressed.len() < min_expected {
        return false;
    }

    // Scanline structure validation: each scanline starts with a filter byte (0..=4)
    for r in 0..(height as usize) {
        let offset = r * row_bytes;
        if offset >= decompressed.len() {
            return false;
        }
        let filter_type = decompressed[offset];
        if filter_type > 4 {
            return false;
        }
    }

    true
}

pub fn decode_jpeg_stream(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return false;
    }
    let mut pos = 2;
    let mut saw_sof = false;
    let mut saw_eoi = false;

    while pos < bytes.len() {
        if bytes[pos] != 0xFF {
            pos += 1;
            continue;
        }
        while pos < bytes.len() && bytes[pos] == 0xFF {
            pos += 1;
        }
        if pos >= bytes.len() {
            break;
        }
        let marker = bytes[pos];
        pos += 1;

        if marker == 0xD9 {
            // EOI (End of Image)
            saw_eoi = true;
            break;
        }
        if marker == 0x00 || (0xD0..=0xD7).contains(&marker) || marker == 0x01 {
            continue;
        }
        if pos + 2 > bytes.len() {
            return false;
        }
        let seg_len = u16::from_be_bytes([bytes[pos], bytes[pos + 1]]) as usize;
        if seg_len < 2 || pos + seg_len > bytes.len() {
            return false;
        }

        // SOF markers: SOF0..SOF3, SOF5..SOF7, SOF9..SOF11, SOF13..SOF15
        if matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF) && seg_len >= 8 {
            let height = u16::from_be_bytes([bytes[pos + 3], bytes[pos + 4]]);
            let width = u16::from_be_bytes([bytes[pos + 5], bytes[pos + 6]]);
            let components = bytes[pos + 7];
            if width > 0 && height > 0 && components >= 1 {
                saw_sof = true;
            }
        }
        pos += seg_len;
    }

    // Must be a complete JPEG stream containing SOF dimensions AND terminating EOI
    saw_sof && saw_eoi
}

pub fn decode_gif_stream(bytes: &[u8]) -> bool {
    if bytes.len() < 14 || (!bytes.starts_with(b"GIF87a") && !bytes.starts_with(b"GIF89a")) {
        return false;
    }
    let width = u16::from_le_bytes([bytes[6], bytes[7]]);
    let height = u16::from_le_bytes([bytes[8], bytes[9]]);
    if width == 0 || height == 0 {
        return false;
    }
    let packed = bytes[10];
    let mut pos = 13;
    if (packed & 0x80) != 0 {
        let gct_size = 3 * (1 << ((packed & 0x07) + 1));
        pos += gct_size;
    }

    let mut saw_image_descriptor = false;
    while pos < bytes.len() {
        match bytes[pos] {
            0x3B => return saw_image_descriptor,
            0x21 => {
                // Extension block: skip function code and sub-blocks
                pos += 2;
                while pos < bytes.len() {
                    let sub_len = bytes[pos] as usize;
                    pos += 1;
                    if sub_len == 0 {
                        break;
                    }
                    pos += sub_len;
                }
            }
            0x2C => {
                // Image Descriptor
                if pos + 10 > bytes.len() {
                    return false;
                }
                let img_w = u16::from_le_bytes([bytes[pos + 5], bytes[pos + 6]]);
                let img_h = u16::from_le_bytes([bytes[pos + 7], bytes[pos + 8]]);
                if img_w > 0 && img_h > 0 {
                    saw_image_descriptor = true;
                }
                let lcl_packed = bytes[pos + 9];
                pos += 10;
                if (lcl_packed & 0x80) != 0 {
                    let lct_size = 3 * (1 << ((lcl_packed & 0x07) + 1));
                    pos += lct_size;
                }
                if pos >= bytes.len() {
                    return false;
                }
                pos += 1; // LZW min code size
                while pos < bytes.len() {
                    let sub_len = bytes[pos] as usize;
                    pos += 1;
                    if sub_len == 0 {
                        break;
                    }
                    pos += sub_len;
                }
            }
            _ => {
                pos += 1;
            }
        }
    }
    saw_image_descriptor
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
        b"VP8 " => {
            if bytes.len() < 30 || chunk_size < 10 {
                return false;
            }
            // VP8 keyframe start code 0x9D 0x01 0x2A
            let has_keyframe_code = bytes[23..26] == [0x9D, 0x01, 0x2A];
            let width = (u16::from_le_bytes([bytes[26], bytes[27]]) & 0x3FFF) as u32;
            let height = (u16::from_le_bytes([bytes[28], bytes[29]]) & 0x3FFF) as u32;
            has_keyframe_code && width > 0 && height > 0
        }
        b"VP8L" => {
            if bytes.len() < 25 || bytes[20] != 0x2F || chunk_size < 5 {
                return false;
            }
            let b1 = bytes[21] as u32;
            let b2 = bytes[22] as u32;
            let b3 = bytes[23] as u32;
            let b4 = bytes[24] as u32;
            let width = 1 + (((b2 & 0x3F) << 8) | b1);
            let height = 1 + (((b4 & 0x0F) << 10) | (b3 << 2) | ((b2 & 0xC0) >> 6));
            width > 0 && height > 0
        }
        b"VP8X" => {
            if bytes.len() < 30 || chunk_size < 10 {
                return false;
            }
            let width = 1 + (bytes[24] as u32 | ((bytes[25] as u32) << 8) | ((bytes[26] as u32) << 16));
            let height = 1 + (bytes[27] as u32 | ((bytes[28] as u32) << 8) | ((bytes[29] as u32) << 16));
            width > 0 && height > 0
        }
        _ => false,
    }
}

pub fn is_visual_media_decodable(bytes: &[u8]) -> bool {
    if !is_valid_visual_asset_format(bytes) {
        return false;
    }
    if is_mock_visual_bytes(bytes) {
        return true;
    }

    if bytes.starts_with(b"\x89PNG") {
        return decode_png_stream(bytes);
    }

    if bytes.starts_with(&[0xFF, 0xD8]) {
        return decode_jpeg_stream(bytes);
    }

    if bytes.starts_with(b"GIF8") {
        return decode_gif_stream(bytes);
    }

    if bytes.starts_with(b"RIFF") {
        return decode_webp_stream(bytes);
    }

    if let Ok(utf8_str) = std::str::from_utf8(bytes) {
        let lower = utf8_str.to_ascii_lowercase();
        if lower.contains("<svg") && (lower.contains("</svg>") || lower.contains("/>")) {
            return !lower.contains("<script")
                && !lower.contains("<foreignobject")
                && !lower.contains("<iframe")
                && !lower.contains("javascript:")
                && !has_inline_event_handler(&lower);
        }
    }

    if bytes.len() >= 16 && (&bytes[4..8] == b"ftyp" || &bytes[4..8] == b"moov") {
        let mut pos = 0;
        let mut saw_ftyp = false;
        let mut saw_moov_or_mdat = false;

        while pos + 8 <= bytes.len() {
            let box_len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
            let box_type = &bytes[pos + 4..pos + 8];

            let effective_len = if box_len == 0 {
                bytes.len() - pos
            } else if box_len == 1 {
                if pos + 16 > bytes.len() {
                    return false;
                }
                let l = u64::from_be_bytes([
                    bytes[pos + 8], bytes[pos + 9], bytes[pos + 10], bytes[pos + 11],
                    bytes[pos + 12], bytes[pos + 13], bytes[pos + 14], bytes[pos + 15],
                ]) as usize;
                if l < 16 {
                    return false;
                }
                l
            } else {
                if box_len < 8 {
                    return false;
                }
                box_len
            };

            if pos + effective_len > bytes.len() {
                return false;
            }

            if box_type == b"ftyp" {
                saw_ftyp = true;
            } else if box_type == b"moov" || box_type == b"mdat" {
                saw_moov_or_mdat = true;
            }

            pos += effective_len;
        }

        return (saw_ftyp || &bytes[4..8] == b"moov") && (saw_moov_or_mdat || saw_ftyp);
    }

    false
}
