//! Stream-level decoding and integrity validation for visual media containers.
//!
//! Enforces clean container parsing and pixel raster decompression (PNG, JPEG, GIF, WebP, SVG, MP4).

pub mod mp4;
pub mod raster;
pub mod webp;

pub use mp4::decode_mp4_stream;
pub use raster::{
    decode_gif_stream, decode_jpeg_stream, decode_png_stream, decode_raster_image,
    validate_svg_content,
};
pub use webp::decode_webp_stream;

use super::visual::{is_mock_visual_bytes, is_valid_visual_asset_format};

pub fn is_visual_media_decodable(bytes: &[u8]) -> bool {
    if is_mock_visual_bytes(bytes) {
        return false;
    }
    if !is_valid_visual_asset_format(bytes) {
        return false;
    }

    if bytes.starts_with(b"\x89PNG") {
        return decode_png_stream(bytes) && decode_raster_image(bytes);
    }
    if bytes.starts_with(&[0xFF, 0xD8]) {
        return decode_jpeg_stream(bytes) && decode_raster_image(bytes);
    }
    if bytes.starts_with(b"GIF8") {
        return decode_gif_stream(bytes) && decode_raster_image(bytes);
    }
    if bytes.starts_with(b"RIFF") {
        return decode_webp_stream(bytes) && decode_raster_image(bytes);
    }
    if let Ok(utf8_str) = std::str::from_utf8(bytes) {
        if validate_svg_content(utf8_str) {
            return true;
        }
    }
    decode_mp4_stream(bytes)
}
