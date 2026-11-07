//! Raster and vector media decoders (PNG, JPEG, GIF, SVG) with resource limits.

use super::super::visual::{has_inline_event_handler, is_mock_visual_bytes};

pub fn decode_png_stream(bytes: &[u8]) -> bool {
    let Some((hdr, compressed_idat)) = parse_png_chunks(bytes) else {
        return false;
    };
    let Some(bits_per_pixel) = calculate_png_bpp(&hdr) else {
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

    validate_png_scanlines(
        &decompressed,
        hdr.width,
        hdr.height,
        bits_per_pixel,
        row_bytes,
        hdr.interlace,
    )
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

fn calculate_png_bpp(hdr: &PngHeader) -> Option<usize> {
    let bpp = match hdr.color_type {
        0 | 3 => 1usize,
        2 => 3usize,
        4 => 2usize,
        6 => 4usize,
        _ => return None,
    };
    bpp.checked_mul(hdr.bit_depth as usize)
}

fn calculate_png_row_bytes(hdr: &PngHeader) -> Option<usize> {
    let bits_per_pixel = calculate_png_bpp(hdr)?;
    let row_data_bits = (hdr.width as usize).checked_mul(bits_per_pixel)?;
    let row_data_bytes = row_data_bits.div_ceil(8);
    1usize.checked_add(row_data_bytes)
}

fn validate_png_scanlines(
    decompressed: &[u8],
    width: u32,
    height: u32,
    bits_per_pixel: usize,
    row_bytes: usize,
    interlace: u8,
) -> bool {
    if interlace == 0 {
        for r in 0..(height as usize) {
            let offset = r * row_bytes;
            if offset >= decompressed.len() || decompressed[offset] > 4 {
                return false;
            }
        }
        true
    } else {
        validate_adam7_scanlines(decompressed, width, height, bits_per_pixel)
    }
}

fn validate_adam7_scanlines(
    decompressed: &[u8],
    width: u32,
    height: u32,
    bits_per_pixel: usize,
) -> bool {
    const PASSES: [(u32, u32, u32, u32); 7] = [
        (0, 0, 8, 8),
        (4, 0, 8, 8),
        (0, 4, 4, 8),
        (2, 0, 4, 4),
        (0, 2, 2, 4),
        (1, 0, 2, 2),
        (0, 1, 1, 2),
    ];
    let mut offset = 0;
    for (x0, y0, dx, dy) in PASSES {
        let pw = if width > x0 { (width - x0).div_ceil(dx) } else { 0 };
        let ph = if height > y0 { (height - y0).div_ceil(dy) } else { 0 };
        if pw == 0 || ph == 0 {
            continue;
        }
        let pass_row_data_bits = (pw as usize) * bits_per_pixel;
        let pass_row_bytes = 1 + pass_row_data_bits.div_ceil(8);
        for _ in 0..ph {
            if offset >= decompressed.len() || decompressed[offset] > 4 {
                return false;
            }
            offset += pass_row_bytes;
        }
    }
    offset <= decompressed.len()
}

fn is_jpeg_sof_marker(marker: u8) -> bool {
    matches!(marker, 0xC0..=0xC3 | 0xC5..=0xC7 | 0xC9..=0xCB | 0xCD..=0xCF)
}

fn parse_jpeg_sof_header(bytes: &[u8], pos: usize, seg_len: usize) -> bool {
    if seg_len < 8 || pos + 8 > bytes.len() {
        return false;
    }
    let height = u16::from_be_bytes([bytes[pos + 3], bytes[pos + 4]]);
    let width = u16::from_be_bytes([bytes[pos + 5], bytes[pos + 6]]);
    let components = bytes[pos + 7];
    width > 0 && height > 0 && (1..=4).contains(&components)
}

fn scan_jpeg_sos_entropy(bytes: &[u8], mut pos: usize) -> Option<(usize, bool)> {
    let mut entropy_bytes = 0;
    let mut saw_eoi = false;

    while pos < bytes.len() {
        if bytes[pos] != 0xFF {
            entropy_bytes += 1;
            pos += 1;
            continue;
        }

        if pos + 1 >= bytes.len() {
            return None;
        }

        let next = bytes[pos + 1];
        if next == 0x00 || (0xD0..=0xD7).contains(&next) {
            entropy_bytes += 1;
            pos += 2;
            continue;
        }

        if next == 0xD9 {
            saw_eoi = true;
            break;
        }

        return None;
    }

    Some((entropy_bytes, saw_eoi))
}

fn next_jpeg_marker(bytes: &[u8], mut pos: usize) -> (usize, Option<u8>) {
    while pos < bytes.len() && bytes[pos] != 0xFF {
        pos += 1;
    }
    while pos < bytes.len() && bytes[pos] == 0xFF {
        pos += 1;
    }
    if pos >= bytes.len() {
        return (pos, None);
    }
    (pos + 1, Some(bytes[pos]))
}

pub fn decode_jpeg_stream(bytes: &[u8]) -> bool {
    if bytes.len() < 4 || bytes[0] != 0xFF || bytes[1] != 0xD8 {
        return false;
    }
    let mut pos = 2;
    let mut saw_sof = false;
    let mut saw_sos = false;
    let mut saw_eoi = false;
    let mut entropy_bytes = 0;

    while pos < bytes.len() {
        let (marker_pos, marker_opt) = next_jpeg_marker(bytes, pos);
        pos = marker_pos;
        let Some(marker) = marker_opt else {
            break;
        };

        if marker == 0xD9 {
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

        if is_jpeg_sof_marker(marker) {
            if parse_jpeg_sof_header(bytes, pos, seg_len) {
                saw_sof = true;
            }
        } else if marker == 0xDA {
            saw_sos = true;
            let Some((scanned_entropy, scanned_eoi)) = scan_jpeg_sos_entropy(bytes, pos + seg_len) else {
                return false;
            };
            entropy_bytes = scanned_entropy;
            saw_eoi = scanned_eoi;
            break;
        }
        pos += seg_len;
    }

    let valid_sos_scan = if saw_sos {
        entropy_bytes > 0 && saw_eoi
    } else {
        saw_eoi
    };
    saw_sof && valid_sos_scan
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

fn collect_gif_lzw_data(bytes: &[u8], mut pos: usize) -> Option<(usize, Vec<u8>)> {
    if pos >= bytes.len() {
        return None;
    }
    let min_code_size = bytes[pos] as usize;
    pos += 1;
    if !(2..=11).contains(&min_code_size) {
        return None;
    }

    let mut lzw_data = Vec::new();
    while pos < bytes.len() {
        let sub_len = bytes[pos] as usize;
        pos += 1;
        if sub_len == 0 {
            break;
        }
        if pos + sub_len > bytes.len() {
            return None;
        }
        lzw_data.extend_from_slice(&bytes[pos..pos + sub_len]);
        pos += sub_len;
    }
    if lzw_data.is_empty() {
        return None;
    }
    Some((min_code_size, lzw_data))
}

fn validate_lzw_codes(min_code_size: usize, lzw_data: &[u8]) -> bool {
    let clear_code = 1usize << min_code_size;
    let eoi_code = clear_code + 1;
    let mut code_size = min_code_size + 1;
    let mut next_code = clear_code + 2;

    let mut bit_buf = 0u32;
    let mut bits_in_buf = 0usize;
    let mut byte_idx = 0usize;
    let mut saw_clear = false;
    let mut saw_eoi = false;

    while byte_idx < lzw_data.len() || bits_in_buf >= code_size {
        while bits_in_buf < code_size {
            if byte_idx >= lzw_data.len() {
                break;
            }
            bit_buf |= (lzw_data[byte_idx] as u32) << bits_in_buf;
            bits_in_buf += 8;
            byte_idx += 1;
        }
        if bits_in_buf < code_size {
            break;
        }
        let mask = (1u32 << code_size) - 1;
        let code = (bit_buf & mask) as usize;
        bit_buf >>= code_size;
        bits_in_buf -= code_size;

        if !saw_clear {
            if code != clear_code {
                return false;
            }
            saw_clear = true;
            continue;
        }

        if code == clear_code {
            code_size = min_code_size + 1;
            next_code = clear_code + 2;
            continue;
        }
        if code == eoi_code {
            saw_eoi = true;
            break;
        }
        if code > next_code || next_code > 4096 {
            return false;
        }
        next_code += 1;
        if next_code == (1usize << code_size) && code_size < 12 {
            code_size += 1;
        }
    }

    saw_clear && saw_eoi
}

fn validate_gif_lzw_stream(bytes: &[u8], pos: usize) -> bool {
    let Some((min_code_size, lzw_data)) = collect_gif_lzw_data(bytes, pos) else {
        return false;
    };
    validate_lzw_codes(min_code_size, &lzw_data)
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
    let lzw_valid = validate_gif_lzw_stream(bytes, next_pos);
    let end_pos = skip_gif_sub_blocks(bytes, next_pos + 1)?;
    Some((end_pos, valid_dim && lzw_valid))
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

pub fn validate_svg_content(utf8_str: &str) -> bool {
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

pub fn decode_raster_image(bytes: &[u8]) -> bool {
    if is_mock_visual_bytes(bytes) {
        return false;
    }
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(10_000);
    limits.max_image_height = Some(10_000);
    limits.max_alloc = Some(64 * 1024 * 1024);
    let mut reader = match image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format() {
        Ok(r) => r,
        Err(_) => return false,
    };
    reader.limits(limits);
    match reader.decode() {
        Ok(img) => img.width() > 0 && img.height() > 0,
        Err(_) => false,
    }
}
