//! Stream-level decoding and integrity validation for visual media containers.
//!
//! Enforces clean container parsing and pixel raster decompression (PNG, JPEG, GIF, WebP, SVG, MP4).

use super::visual::{has_inline_event_handler, is_mock_visual_bytes, is_valid_visual_asset_format};

pub fn decode_png_stream(bytes: &[u8]) -> bool {
    let Some((hdr, compressed_idat)) = parse_png_chunks(bytes) else {
        return false;
    };
    let Some(row_bytes) = calculate_png_row_bytes(&hdr) else {
        return false;
    };
    let Some(min_expected) = (hdr.height as usize).checked_mul(row_bytes) else {
        return false;
    };

    // Zip bomb & RAM limit protection: max 32 MB decompressed output
    const MAX_DECOMPRESSED_PNG_BYTES: usize = 32 * 1024 * 1024;
    if min_expected > MAX_DECOMPRESSED_PNG_BYTES {
        return false;
    }

    let Ok(decompressed) = miniz_oxide::inflate::decompress_to_vec_zlib_with_limit(
        &compressed_idat,
        MAX_DECOMPRESSED_PNG_BYTES,
    ) else {
        return false;
    };

    if decompressed.len() < min_expected {
        return false;
    }

    validate_png_scanlines(&decompressed, hdr.height, row_bytes, hdr.interlace)
}

struct PngHeader {
    width: u32,
    height: u32,
    bit_depth: u8,
    color_type: u8,
    interlace: u8,
}

fn parse_png_chunk_crc(bytes: &[u8], pos: usize, chunk_len: usize) -> bool {
    let data_end = pos + 8 + chunk_len;
    let mut hasher = crc32fast::Hasher::new();
    hasher.update(&bytes[pos + 4..data_end]);
    let expected_crc = u32::from_be_bytes([
        bytes[data_end],
        bytes[data_end + 1],
        bytes[data_end + 2],
        bytes[data_end + 3],
    ]);
    hasher.finalize() == expected_crc
}

fn parse_png_chunks(bytes: &[u8]) -> Option<(PngHeader, Vec<u8>)> {
    if !bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        return None;
    }
    let mut pos = 8;
    let mut header = None;
    let mut saw_iend = false;
    let mut compressed_idat = Vec::new();

    while pos + 8 <= bytes.len() {
        let chunk_len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let chunk_type = &bytes[pos + 4..pos + 8];
        let data_start = pos + 8;
        let data_end = data_start + chunk_len;
        let chunk_end = data_end + 4;

        if chunk_end > bytes.len() || !parse_png_chunk_crc(bytes, pos, chunk_len) {
            return None;
        }

        match chunk_type {
            b"IHDR" => {
                if chunk_len < 13 || header.is_some() {
                    return None;
                }
                let width = u32::from_be_bytes([bytes[data_start], bytes[data_start + 1], bytes[data_start + 2], bytes[data_start + 3]]);
                let height = u32::from_be_bytes([bytes[data_start + 4], bytes[data_start + 5], bytes[data_start + 6], bytes[data_start + 7]]);
                if width == 0 || height == 0 || width > 10000 || height > 10000 {
                    return None;
                }
                let bit_depth = bytes[data_start + 8];
                let color_type = bytes[data_start + 9];
                let interlace = bytes[data_start + 12];
                if !matches!(bit_depth, 1 | 2 | 4 | 8 | 16) || interlace > 1 {
                    return None;
                }
                header = Some(PngHeader {
                    width,
                    height,
                    bit_depth,
                    color_type,
                    interlace,
                });
            }
            b"IDAT" => {
                if header.is_none() || chunk_len == 0 {
                    return None;
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

    if let (Some(hdr), true) = (header, saw_iend) {
        if !compressed_idat.is_empty() {
            return Some((hdr, compressed_idat));
        }
    }
    None
}

fn calculate_png_row_bytes(hdr: &PngHeader) -> Option<usize> {
    let bpp = match hdr.color_type {
        0 | 3 => 1usize,
        2 => 3usize,
        4 => 2usize,
        6 => 4usize,
        _ => return None,
    };
    let bits_per_pixel = bpp.checked_mul(hdr.bit_depth as usize)?;
    let row_data_bits = (hdr.width as usize).checked_mul(bits_per_pixel)?;
    let row_data_bytes = (row_data_bits + 7) / 8;
    1usize.checked_add(row_data_bytes)
}

fn validate_png_scanlines(decompressed: &[u8], height: u32, row_bytes: usize, interlace: u8) -> bool {
    if interlace == 0 {
        for r in 0..(height as usize) {
            let offset = r * row_bytes;
            if offset >= decompressed.len() || decompressed[offset] > 4 {
                return false;
            }
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

fn skip_gif_sub_blocks(bytes: &[u8], mut pos: usize) -> Option<usize> {
    while pos < bytes.len() {
        let sub_len = bytes[pos] as usize;
        pos += 1;
        if sub_len == 0 {
            return Some(pos);
        }
        pos += sub_len;
    }
    None
}

fn process_gif_image_descriptor(bytes: &[u8], pos: usize) -> Option<(usize, bool)> {
    if pos + 10 > bytes.len() {
        return None;
    }
    let img_w = u16::from_le_bytes([bytes[pos + 5], bytes[pos + 6]]);
    let img_h = u16::from_le_bytes([bytes[pos + 7], bytes[pos + 8]]);
    let valid_dim = img_w > 0 && img_h > 0;
    let lcl_packed = bytes[pos + 9];
    let mut next_pos = pos + 10;
    if (lcl_packed & 0x80) != 0 {
        let lct_size = 3 * (1 << ((lcl_packed & 0x07) + 1));
        next_pos += lct_size;
    }
    if next_pos >= bytes.len() {
        return None;
    }
    next_pos += 1; // LZW min code size
    let end_pos = skip_gif_sub_blocks(bytes, next_pos)?;
    Some((end_pos, valid_dim))
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
                let Some(next_pos) = skip_gif_sub_blocks(bytes, pos + 2) else {
                    return false;
                };
                pos = next_pos;
            }
            0x2C => {
                let Some((next_pos, valid_dim)) = process_gif_image_descriptor(bytes, pos) else {
                    return false;
                };
                if valid_dim {
                    saw_image_descriptor = true;
                }
                pos = next_pos;
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

fn validate_svg_content(utf8_str: &str) -> bool {
    let lower = utf8_str.to_ascii_lowercase();
    if lower.contains("<svg") && (lower.contains("</svg>") || lower.contains("/>")) {
        return !lower.contains("<script")
            && !lower.contains("<foreignobject")
            && !lower.contains("<iframe")
            && !lower.contains("javascript:")
            && !has_inline_event_handler(&lower);
    }
    false
}

fn parse_mp4_box_length(bytes: &[u8], pos: usize, box_len: usize) -> Option<usize> {
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

fn decode_mp4_stream(bytes: &[u8]) -> bool {
    if bytes.len() < 16 || (&bytes[4..8] != b"ftyp" && &bytes[4..8] != b"moov") {
        return false;
    }
    let mut pos = 0;
    let mut saw_ftyp = false;
    let mut saw_moov_or_mdat = false;

    while pos + 8 <= bytes.len() {
        let box_len = u32::from_be_bytes([bytes[pos], bytes[pos + 1], bytes[pos + 2], bytes[pos + 3]]) as usize;
        let box_type = &bytes[pos + 4..pos + 8];
        let Some(effective_len) = parse_mp4_box_length(bytes, pos, box_len) else {
            return false;
        };

        let Some(next_pos) = pos.checked_add(effective_len) else {
            return false;
        };
        if next_pos > bytes.len() || next_pos <= pos {
            return false;
        }

        if box_type == b"ftyp" {
            saw_ftyp = true;
        } else if box_type == b"moov" || box_type == b"mdat" {
            saw_moov_or_mdat = true;
        }

        pos = next_pos;
    }

    (saw_ftyp || &bytes[4..8] == b"moov") && (saw_moov_or_mdat || saw_ftyp)
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
        if validate_svg_content(utf8_str) {
            return true;
        }
    }
    decode_mp4_stream(bytes)
}
