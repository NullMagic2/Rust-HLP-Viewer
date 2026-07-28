//! Classic WinHelp graphics resource decoding.
//!
//! Topic picture commands either reference an internal `|bmN` stream or carry an embedded
//! graphics stream. Logical graphics objects may contain DIB/DDB raster alternatives or a legacy
//! Windows metafile, and may carry clickable rectangular hotspots. Bitmap/metafile payloads share
//! the historical WinHelp packing modes (raw, RLE, LZ77, or LZ77+RLE). The HLP engine always
//! exposes top-down RGBA; Windows WMF playback is delegated to the narrow `wmf-render` adapter.

use crate::compression::lz77_decompress;
use crate::encoding::decode_windows_1252;
use crate::reader::Reader;
use crate::{HlpError, HlpFile};
use std::sync::Arc;

const MAX_GRAPHICS_BYTES: usize = 64 * 1024 * 1024;
const MAX_DIMENSION: u32 = 16_384;
const MAX_PIXELS: usize = MAX_GRAPHICS_BYTES / 4;
const MAX_GRAPHICAL_HOTSPOTS: usize = 4_096;
const RASTERIZATION_DPI: u32 = 96;

/// Authored physical-size metadata retained separately from the decoded raster pixels.
///
/// KB917607's picture-size helper at `0x40661A..0x4066F6` uses the current device DPI when
/// bitmap resolution fields or physical WMF mapping modes are present. Keeping this metadata on
/// the decoded image lets retained layout reproduce that calculation without resampling the
/// source simply because the host DPI changes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PictureSizing {
    /// No authored physical size: one source pixel remains one layout pixel.
    Pixels,
    /// Bitmap resolution fields used as pixels-per-inch divisors by WinHlp32.
    BitmapResolution {
        x_resolution: u32,
        y_resolution: u32,
    },
    /// WMF logical extents whose mapping mode defines their physical units.
    Metafile {
        mapping_mode: i32,
        logical_width: u32,
        logical_height: u32,
    },
}

/// Unresolved action stored in a graphical-hotspot table. Context names are resolved only after
/// the document's navigation metadata has been loaded.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum DecodedPictureHotspotTarget {
    Macro(String),
    Context {
        name: String,
        popup: bool,
        window_name: Option<String>,
        opcode: u8,
    },
}

/// One clickable source-space rectangle carried by a WinHelp picture resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct DecodedPictureHotspot {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
    pub target: DecodedPictureHotspotTarget,
}

/// GUI-independent decoded raster image, stored as top-down RGBA pixels.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DecodedPicture {
    pub width: u32,
    pub height: u32,
    pub rgba: Arc<[u8]>,
    /// True when at least one decoded pixel is transparent.
    pub has_alpha: bool,
    pub(crate) sizing: PictureSizing,
    pub(crate) hotspots: Arc<[DecodedPictureHotspot]>,
}

/// Loads and decodes a logical WinHelp bitmap from an internal `|bmN` stream.
pub(crate) fn decode_indexed_picture(file: &HlpFile, index: u16) -> Result<DecodedPicture, HlpError> {
    let name = format!("|bm{index}");
    let stream = file.internal_file(&name)?;
    decode_graphics_stream(stream.content)
}

/// Decodes an embedded WinHelp logical graphics stream.
pub(crate) fn decode_embedded_picture(bytes: &[u8]) -> Result<DecodedPicture, HlpError> {
    // Known WinHelp writers leave two trailing bytes outside the logical graphics object while the
    // topic command's SizeOfPicture includes them. Try the exact payload first, then that canonical
    // trimmed form so both writer conventions remain accepted.
    match decode_graphics_stream(bytes) {
        Ok(image) => Ok(image),
        Err(primary) if bytes.len() >= 2 => decode_graphics_stream(&bytes[..bytes.len() - 2])
            .map_err(|_| primary),
        Err(error) => Err(error),
    }
}

/// Decodes the first usable bitmap alternative in a WinHelp graphics stream.
fn decode_graphics_stream(bytes: &[u8]) -> Result<DecodedPicture, HlpError> {
    if bytes.len() > MAX_GRAPHICS_BYTES {
        return Err(HlpError::invalid(
            "WinHelp graphics stream",
            format!("{} bytes exceeds safety limit {MAX_GRAPHICS_BYTES}", bytes.len()),
        ));
    }
    if bytes.len() < 8 {
        return Err(HlpError::UnexpectedEof {
            context: "WinHelp graphics header",
        });
    }

    let picture_count = usize::from(u16::from_le_bytes([bytes[2], bytes[3]]));
    if picture_count == 0 {
        return Err(HlpError::invalid("WinHelp graphics stream", "contains no picture alternatives"));
    }
    let table_end = 4_usize
        .checked_add(picture_count.checked_mul(4).ok_or_else(|| {
            HlpError::invalid("WinHelp graphics stream", "picture-offset table overflow")
        })?)
        .ok_or_else(|| HlpError::invalid("WinHelp graphics stream", "picture-offset table overflow"))?;
    if table_end > bytes.len() {
        return Err(HlpError::UnexpectedEof {
            context: "WinHelp graphics offset table",
        });
    }

    let mut alternatives = Vec::with_capacity(picture_count);
    for index in 0..picture_count {
        let offset_pos = 4 + index * 4;
        let raw_offset = bytes.get(offset_pos..offset_pos + 4).ok_or(HlpError::UnexpectedEof {
            context: "WinHelp graphics offset table",
        })?;
        let offset = usize::try_from(u32::from_le_bytes([
            raw_offset[0],
            raw_offset[1],
            raw_offset[2],
            raw_offset[3],
        ]))
        .map_err(|_| HlpError::invalid("WinHelp graphics stream", "picture offset does not fit usize"))?;
        let header = bytes.get(offset..offset.saturating_add(2)).ok_or_else(|| {
            HlpError::invalid(
                "WinHelp graphics stream",
                format!("picture alternative {index} starts outside the stream at {offset}"),
            )
        })?;
        alternatives.push((offset, header[0], header[1]));
    }

    // Prefer a DIB because it carries an explicit palette, then a self-describing DDB, and finally
    // the vector WMF alternative. A malformed preferred alternative must not hide a valid fallback.
    let mut last_error = None;
    for wanted_type in [0x06_u8, 0x05_u8, 0x08_u8] {
        for &(offset, picture_type, packing) in &alternatives {
            if picture_type != wanted_type {
                continue;
            }
            let decoded = if picture_type == 0x08 {
                decode_metafile_record(bytes, offset, packing)
            } else {
                decode_bitmap_record(bytes, offset, picture_type, packing)
            };
            match decoded {
                Ok(image) => return Ok(image),
                Err(error) => last_error = Some(error),
            }
        }
    }

    if let Some(error) = last_error {
        return Err(error);
    }
    let types = alternatives
        .iter()
        .map(|(_, kind, _)| format!("0x{kind:02X}"))
        .collect::<Vec<_>>()
        .join(", ");
    Err(HlpError::Unsupported {
        context: "WinHelp graphics type",
        detail: format!("no supported bitmap or metafile alternative is present (types: {types})"),
    })
}

fn decode_bitmap_record(
    stream: &[u8],
    record_offset: usize,
    picture_type: u8,
    packing: u8,
) -> Result<DecodedPicture, HlpError> {
    let record = stream.get(record_offset..).ok_or_else(|| {
        HlpError::invalid("WinHelp bitmap", "bitmap record offset is outside graphics stream")
    })?;
    let mut reader = Reader::new(record, "WinHelp bitmap record");
    let actual_type = reader.read_u8()?;
    let actual_packing = reader.read_u8()?;
    if actual_type != picture_type || actual_packing != packing {
        return Err(HlpError::invalid("WinHelp bitmap", "bitmap alternative header changed while decoding"));
    }

    // These two historical fields are often documented as pels-per-meter by analogy with a
    // BITMAPINFOHEADER. WinHlp32 does not use them that way. Its retained size path divides the
    // source pixel dimensions directly by these values after multiplying by LOGPIXELSX/Y, so
    // preserve them as authored pixels-per-inch resolution values.
    let x_resolution = read_compressed_unsigned_long(&mut reader)?;
    let y_resolution = read_compressed_unsigned_long(&mut reader)?;
    let planes = read_compressed_unsigned_short(&mut reader)?;
    let bit_count = read_compressed_unsigned_short(&mut reader)?;
    let width = read_compressed_unsigned_long(&mut reader)?;
    let height = read_compressed_unsigned_long(&mut reader)?;
    let clr_used = read_compressed_unsigned_long(&mut reader)?;
    let clr_important = read_compressed_unsigned_long(&mut reader)?;
    let compressed_size = usize::try_from(read_compressed_unsigned_long(&mut reader)?)
        .map_err(|_| HlpError::invalid("WinHelp bitmap", "compressed size does not fit usize"))?;
    let hotspot_size = read_compressed_unsigned_long(&mut reader)?;
    let bits_offset = usize::try_from(reader.read_u32()?)
        .map_err(|_| HlpError::invalid("WinHelp bitmap", "pixel offset does not fit usize"))?;
    let hotspot_offset = reader.read_u32()?;

    validate_dimensions(width, height)?;
    if planes != 1 {
        return Err(HlpError::Unsupported {
            context: "WinHelp bitmap",
            detail: format!("{planes} planes; only one-plane bitmap records are supported"),
        });
    }
    if !matches!(bit_count, 1 | 4 | 8 | 16 | 24 | 32) {
        return Err(HlpError::Unsupported {
            context: "WinHelp bitmap",
            detail: format!("{bit_count}-bit pixels are not supported"),
        });
    }

    let palette_count = if picture_type == 0x06 && bit_count <= 8 {
        let maximum_palette_count = 1_u32 << bit_count;
        let count = if clr_used == 0 {
            maximum_palette_count
        } else {
            clr_used
        };
        if count > maximum_palette_count {
            return Err(HlpError::invalid(
                "WinHelp bitmap palette",
                format!(
                    "{count} entries exceed the {maximum_palette_count}-entry limit for {bit_count}-bit pixels"
                ),
            ));
        }
        count
    } else {
        0
    };
    let palette_count = usize::try_from(palette_count)
        .map_err(|_| HlpError::invalid("WinHelp bitmap palette", "entry count does not fit usize"))?;
    let mut palette = Vec::with_capacity(palette_count);
    for _ in 0..palette_count {
        let bgra = reader.read_bytes(4)?;
        palette.push([bgra[2], bgra[1], bgra[0]]);
    }

    if picture_type == 0x05 && matches!(bit_count, 4 | 8) {
        return Err(HlpError::Unsupported {
            context: "WinHelp DDB bitmap",
            detail: format!("portable palette information is absent for {bit_count}-bit DDB data"),
        });
    }

    let row_bits = usize::try_from(width)
        .map_err(|_| HlpError::invalid("WinHelp bitmap", "width does not fit usize"))?
        .checked_mul(usize::from(bit_count))
        .ok_or_else(|| HlpError::invalid("WinHelp bitmap", "row bit count overflow"))?;
    let row_stride = row_bits
        .checked_add(31)
        .ok_or_else(|| HlpError::invalid("WinHelp bitmap", "row stride overflow"))?
        / 32
        * 4;
    let decoded_size = row_stride
        .checked_mul(usize::try_from(height).map_err(|_| {
            HlpError::invalid("WinHelp bitmap", "height does not fit usize")
        })?)
        .ok_or_else(|| HlpError::invalid("WinHelp bitmap", "decoded pixel size overflow"))?;
    if decoded_size > MAX_GRAPHICS_BYTES {
        return Err(HlpError::invalid(
            "WinHelp bitmap",
            format!("decoded bitmap size {decoded_size} exceeds {MAX_GRAPHICS_BYTES} bytes"),
        ));
    }

    let source_start = record_offset
        .checked_add(bits_offset)
        .ok_or_else(|| HlpError::invalid("WinHelp bitmap", "pixel source offset overflow"))?;
    let source_end = source_start
        .checked_add(compressed_size)
        .ok_or_else(|| HlpError::invalid("WinHelp bitmap", "compressed pixel range overflow"))?;
    let source = stream.get(source_start..source_end).ok_or_else(|| {
        HlpError::invalid(
            "WinHelp bitmap",
            format!("compressed pixel range {source_start}..{source_end} lies outside graphics stream"),
        )
    })?;
    let pixels = decompress_graphics(source, packing, decoded_size)?;

    let mut image = decode_pixels_to_rgba(
        &pixels,
        width,
        height,
        bit_count,
        row_stride,
        &palette,
        clr_important == 1 && !palette.is_empty(),
        picture_type,
    )?;
    image.sizing = if x_resolution != 0 && y_resolution != 0 {
        PictureSizing::BitmapResolution {
            x_resolution,
            y_resolution,
        }
    } else {
        PictureSizing::Pixels
    };
    image.hotspots = Arc::from(parse_graphical_hotspots(record, hotspot_size, hotspot_offset)?);
    Ok(image)
}

fn decode_metafile_record(
    stream: &[u8],
    record_offset: usize,
    packing: u8,
) -> Result<DecodedPicture, HlpError> {
    let record = stream.get(record_offset..).ok_or_else(|| {
        HlpError::invalid("WinHelp metafile", "metafile record offset is outside graphics stream")
    })?;
    let mut reader = Reader::new(record, "WinHelp metafile record");
    let actual_type = reader.read_u8()?;
    let actual_packing = reader.read_u8()?;
    if actual_type != 0x08 || actual_packing != packing {
        return Err(HlpError::invalid(
            "WinHelp metafile",
            "metafile alternative header changed while decoding",
        ));
    }

    let mapping_mode = i32::from(read_compressed_unsigned_short(&mut reader)?);
    let logical_width = u32::from(reader.read_u16()?);
    let logical_height = u32::from(reader.read_u16()?);
    if logical_width == 0 || logical_height == 0 {
        return Err(HlpError::invalid(
            "WinHelp metafile",
            format!("invalid logical dimensions {logical_width}x{logical_height}"),
        ));
    }
    let decompressed_size = usize::try_from(read_compressed_unsigned_long(&mut reader)?)
        .map_err(|_| HlpError::invalid("WinHelp metafile", "decoded size does not fit usize"))?;
    let compressed_size = usize::try_from(read_compressed_unsigned_long(&mut reader)?)
        .map_err(|_| HlpError::invalid("WinHelp metafile", "compressed size does not fit usize"))?;
    let hotspot_size = read_compressed_unsigned_long(&mut reader)?;
    let bits_offset = usize::try_from(reader.read_u32()?)
        .map_err(|_| HlpError::invalid("WinHelp metafile", "metafile offset does not fit usize"))?;
    let hotspot_offset = reader.read_u32()?;
    if decompressed_size == 0 || decompressed_size > MAX_GRAPHICS_BYTES {
        return Err(HlpError::invalid(
            "WinHelp metafile",
            format!("decoded metafile size {decompressed_size} is outside the safety limit"),
        ));
    }

    let source_end = bits_offset
        .checked_add(compressed_size)
        .ok_or_else(|| HlpError::invalid("WinHelp metafile", "compressed range overflow"))?;
    let source = record.get(bits_offset..source_end).ok_or_else(|| {
        HlpError::invalid(
            "WinHelp metafile",
            format!("compressed range {bits_offset}..{source_end} lies outside the graphics record"),
        )
    })?;
    let metafile = decompress_graphics(source, packing, decompressed_size)?;
    // Decode WMF into a stable 96-DPI raster. Retained layout separately derives its natural
    // display size from the authored mapping mode and the actual target device DPI.
    let (width, height) = metafile_pixel_dimensions(
        mapping_mode,
        logical_width,
        logical_height,
        RASTERIZATION_DPI,
        RASTERIZATION_DPI,
    );
    validate_dimensions(width, height)?;
    let rendered = wmf_render::render_windows_metafile(
        &metafile,
        width,
        height,
        mapping_mode,
        i32::try_from(logical_width).map_err(|_| HlpError::invalid("WinHelp metafile", "x extent exceeds i32"))?,
        i32::try_from(logical_height).map_err(|_| HlpError::invalid("WinHelp metafile", "y extent exceeds i32"))?,
    )
        .map_err(|detail| HlpError::Unsupported {
            context: "WinHelp Windows metafile",
            detail,
        })?;
    let expected_rgba = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .and_then(|pixels| pixels.checked_mul(4))
        .ok_or_else(|| HlpError::invalid("WinHelp metafile", "RGBA size overflow"))?;
    if rendered.rgba.len() != expected_rgba {
        return Err(HlpError::invalid(
            "WinHelp metafile",
            format!("renderer returned {} RGBA bytes, expected {expected_rgba}", rendered.rgba.len()),
        ));
    }
    let hotspots = parse_graphical_hotspots(record, hotspot_size, hotspot_offset)?
        .into_iter()
        .map(|hotspot| scale_hotspot(hotspot, logical_width, logical_height, width, height))
        .collect::<Vec<_>>();
    Ok(DecodedPicture {
        width,
        height,
        rgba: Arc::from(rendered.rgba),
        has_alpha: false,
        sizing: PictureSizing::Metafile {
            mapping_mode,
            logical_width,
            logical_height,
        },
        hotspots: Arc::from(hotspots),
    })
}

pub(crate) fn metafile_pixel_dimensions(
    mapping_mode: i32,
    width: u32,
    height: u32,
    dpi_x: u32,
    dpi_y: u32,
) -> (u32, u32) {
    fn convert(value: u32, numerator: u64, denominator: u64) -> u32 {
        let scaled = u64::from(value)
            .saturating_mul(numerator)
            .saturating_add(denominator / 2)
            / denominator;
        u32::try_from(scaled).unwrap_or(u32::MAX).max(1)
    }
    match mapping_mode {
        1 => (width.max(1), height.max(1)), // MM_TEXT: logical unit == device pixel.
        2 => (convert(width, u64::from(dpi_x), 254), convert(height, u64::from(dpi_y), 254)),
        3 => (convert(width, u64::from(dpi_x), 2_540), convert(height, u64::from(dpi_y), 2_540)),
        4 => (convert(width, u64::from(dpi_x), 100), convert(height, u64::from(dpi_y), 100)),
        5 => (convert(width, u64::from(dpi_x), 1_000), convert(height, u64::from(dpi_y), 1_000)),
        6 => (convert(width, u64::from(dpi_x), 1_440), convert(height, u64::from(dpi_y), 1_440)),
        // KB917607 helper 0x4072C7 handles MM_ISOTROPIC/MM_ANISOTROPIC specially with
        // MulDiv(extent, LOGPIXELS*, 2540), i.e. retained METAFILEPICT extents are HIMETRIC.
        7 | 8 => (convert(width, u64::from(dpi_x), 2_540), convert(height, u64::from(dpi_y), 2_540)),
        _ => (width.max(1), height.max(1)),
    }
}

fn scale_hotspot(
    mut hotspot: DecodedPictureHotspot,
    source_width: u32,
    source_height: u32,
    target_width: u32,
    target_height: u32,
) -> DecodedPictureHotspot {
    fn scale(value: u32, source: u32, target: u32) -> u32 {
        u32::try_from(
            u64::from(value)
                .saturating_mul(u64::from(target))
                .checked_div(u64::from(source.max(1)))
                .unwrap_or(0),
        )
        .unwrap_or(u32::MAX)
    }
    hotspot.x = scale(hotspot.x, source_width, target_width);
    hotspot.y = scale(hotspot.y, source_height, target_height);
    hotspot.width = scale(hotspot.width, source_width, target_width).max(1);
    hotspot.height = scale(hotspot.height, source_height, target_height).max(1);
    hotspot
}

fn parse_graphical_hotspots(
    record: &[u8],
    hotspot_size: u32,
    hotspot_offset: u32,
) -> Result<Vec<DecodedPictureHotspot>, HlpError> {
    if hotspot_size == 0 || hotspot_offset == 0 {
        return Ok(Vec::new());
    }
    let start = usize::try_from(hotspot_offset)
        .map_err(|_| HlpError::invalid("WinHelp graphical hotspots", "offset does not fit usize"))?;
    let size = usize::try_from(hotspot_size)
        .map_err(|_| HlpError::invalid("WinHelp graphical hotspots", "size does not fit usize"))?;
    let end = start
        .checked_add(size)
        .ok_or_else(|| HlpError::invalid("WinHelp graphical hotspots", "range overflow"))?;
    let data = record.get(start..end).ok_or_else(|| {
        HlpError::invalid(
            "WinHelp graphical hotspots",
            format!("hotspot range {start}..{end} lies outside the graphics record"),
        )
    })?;
    if data.len() < 7 {
        return Err(HlpError::UnexpectedEof {
            context: "WinHelp graphical-hotspot header",
        });
    }
    let count = usize::from(u16::from_le_bytes([data[1], data[2]]));
    if count > MAX_GRAPHICAL_HOTSPOTS {
        return Err(HlpError::invalid(
            "WinHelp graphical hotspots",
            format!("{count} entries exceed safety limit {MAX_GRAPHICAL_HOTSPOTS}"),
        ));
    }
    let macro_bytes = usize::try_from(u32::from_le_bytes([data[3], data[4], data[5], data[6]]))
        .map_err(|_| HlpError::invalid("WinHelp graphical hotspots", "macro-data size does not fit usize"))?;
    let records_end = 7_usize
        .checked_add(count.checked_mul(15).ok_or_else(|| {
            HlpError::invalid("WinHelp graphical hotspots", "record-table size overflow")
        })?)
        .ok_or_else(|| HlpError::invalid("WinHelp graphical hotspots", "record-table size overflow"))?;
    let mut string_position = records_end
        .checked_add(macro_bytes)
        .ok_or_else(|| HlpError::invalid("WinHelp graphical hotspots", "string-table offset overflow"))?;
    if string_position > data.len() {
        return Err(HlpError::UnexpectedEof {
            context: "WinHelp graphical-hotspot table",
        });
    }

    let mut result = Vec::with_capacity(count);
    for index in 0..count {
        let base = 7 + index * 15;
        let entry = data.get(base..base + 15).ok_or(HlpError::UnexpectedEof {
            context: "WinHelp graphical-hotspot entry",
        })?;
        let opcode = entry[0];
        let x = u32::from(u16::from_le_bytes([entry[3], entry[4]]));
        let y = u32::from(u16::from_le_bytes([entry[5], entry[6]]));
        let width = u32::from(u16::from_le_bytes([entry[7], entry[8]]));
        let height = u32::from(u16::from_le_bytes([entry[9], entry[10]]));
        let _name = read_hotspot_string(data, &mut string_position)?;
        let link = read_hotspot_string(data, &mut string_position)?;
        if width == 0 || height == 0 {
            continue;
        }
        let target = match opcode {
            0xC8 => DecodedPictureHotspotTarget::Macro(link),
            0xE6 | 0xE7 => DecodedPictureHotspotTarget::Context {
                name: link,
                popup: opcode & 1 == 0,
                window_name: None,
                opcode,
            },
            0xEE | 0xEF => {
                let (name, window_name) = link.split_once('>').map_or_else(
                    || (link.clone(), None),
                    |(name, window)| (name.to_owned(), (!window.is_empty()).then(|| window.to_owned())),
                );
                DecodedPictureHotspotTarget::Context {
                    name,
                    popup: opcode & 1 == 0,
                    window_name,
                    opcode,
                }
            }
            _ => continue,
        };
        result.push(DecodedPictureHotspot {
            x,
            y,
            width,
            height,
            target,
        });
    }
    Ok(result)
}

fn read_hotspot_string(data: &[u8], position: &mut usize) -> Result<String, HlpError> {
    let rest = data.get(*position..).ok_or(HlpError::UnexpectedEof {
        context: "WinHelp graphical-hotspot string",
    })?;
    let end = rest.iter().position(|byte| *byte == 0).ok_or(HlpError::UnexpectedEof {
        context: "WinHelp graphical-hotspot string",
    })?;
    let text = decode_windows_1252(&rest[..end]);
    *position = position
        .checked_add(end + 1)
        .ok_or_else(|| HlpError::invalid("WinHelp graphical hotspots", "string offset overflow"))?;
    Ok(text)
}

fn validate_dimensions(width: u32, height: u32) -> Result<(), HlpError> {
    if width == 0 || height == 0 {
        return Err(HlpError::invalid("WinHelp bitmap", format!("invalid dimensions {width}x{height}")));
    }
    if width > MAX_DIMENSION || height > MAX_DIMENSION {
        return Err(HlpError::invalid(
            "WinHelp bitmap",
            format!("dimensions {width}x{height} exceed {MAX_DIMENSION}px safety limit"),
        ));
    }
    let pixels = usize::try_from(width)
        .ok()
        .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
        .ok_or_else(|| HlpError::invalid("WinHelp bitmap", "pixel count overflow"))?;
    if pixels > MAX_PIXELS {
        return Err(HlpError::invalid(
            "WinHelp bitmap",
            format!("pixel count {pixels} exceeds safety limit {MAX_PIXELS}"),
        ));
    }
    Ok(())
}

fn decompress_graphics(input: &[u8], packing: u8, expected: usize) -> Result<Vec<u8>, HlpError> {
    let decoded = match packing {
        0 => {
            if input.len() < expected {
                return Err(HlpError::UnexpectedEof {
                    context: "uncompressed WinHelp bitmap pixels",
                });
            }
            input[..expected].to_vec()
        }
        1 => rle_decompress(input, expected)?,
        2 => lz77_decompress(input, expected)?,
        3 => {
            // The intermediate LZ77 output is itself the RLE stream. Its exact length is not stored,
            // so allow the same bounded maximum as the graphics object and stop when LZ77 ends.
            let rle_stream = lz77_decompress(input, MAX_GRAPHICS_BYTES)?;
            rle_decompress(&rle_stream, expected)?
        }
        other => {
            return Err(HlpError::Unsupported {
                context: "WinHelp graphics packing",
                detail: format!("packing mode {other}"),
            });
        }
    };
    if decoded.len() != expected {
        return Err(HlpError::invalid(
            "WinHelp graphics packing",
            format!("decoded {} bytes, expected {expected}", decoded.len()),
        ));
    }
    Ok(decoded)
}

fn rle_decompress(input: &[u8], expected: usize) -> Result<Vec<u8>, HlpError> {
    let mut output = Vec::with_capacity(expected);
    let mut position = 0_usize;
    while position < input.len() && output.len() < expected {
        let control = input[position];
        position += 1;
        let count = usize::from(control & 0x7F);
        if control & 0x80 != 0 {
            let end = position
                .checked_add(count)
                .ok_or_else(|| HlpError::invalid("WinHelp graphics RLE", "literal range overflow"))?;
            let bytes = input.get(position..end).ok_or(HlpError::UnexpectedEof {
                context: "WinHelp graphics RLE literal run",
            })?;
            if output.len().saturating_add(bytes.len()) > expected {
                return Err(HlpError::invalid("WinHelp graphics RLE", "literal run exceeds decoded size"));
            }
            output.extend_from_slice(bytes);
            position = end;
        } else {
            let value = *input.get(position).ok_or(HlpError::UnexpectedEof {
                context: "WinHelp graphics RLE repeated run",
            })?;
            position += 1;
            if output.len().saturating_add(count) > expected {
                return Err(HlpError::invalid("WinHelp graphics RLE", "repeated run exceeds decoded size"));
            }
            output.resize(output.len() + count, value);
        }
    }
    if output.len() != expected {
        return Err(HlpError::invalid(
            "WinHelp graphics RLE",
            format!("decoded {} bytes, expected {expected}", output.len()),
        ));
    }
    Ok(output)
}

#[allow(clippy::too_many_arguments)]
fn decode_pixels_to_rgba(
    pixels: &[u8],
    width: u32,
    height: u32,
    bit_count: u16,
    row_stride: usize,
    palette: &[[u8; 3]],
    transparent_last_palette_entry: bool,
    picture_type: u8,
) -> Result<DecodedPicture, HlpError> {
    let width_usize = usize::try_from(width)
        .map_err(|_| HlpError::invalid("WinHelp bitmap", "width does not fit usize"))?;
    let height_usize = usize::try_from(height)
        .map_err(|_| HlpError::invalid("WinHelp bitmap", "height does not fit usize"))?;
    let rgba_len = width_usize
        .checked_mul(height_usize)
        .and_then(|value| value.checked_mul(4))
        .ok_or_else(|| HlpError::invalid("WinHelp bitmap", "RGBA buffer size overflow"))?;
    let mut rgba = vec![0_u8; rgba_len];
    let transparent_index = transparent_last_palette_entry.then(|| palette.len() - 1);
    let mut has_alpha = false;

    for output_y in 0..height_usize {
        // Classic WinHelp bitmap records use Windows bottom-up scan lines.
        let source_y = height_usize - 1 - output_y;
        let row_start = source_y
            .checked_mul(row_stride)
            .ok_or_else(|| HlpError::invalid("WinHelp bitmap", "row offset overflow"))?;
        let row = pixels.get(row_start..row_start + row_stride).ok_or(HlpError::UnexpectedEof {
            context: "WinHelp bitmap scan line",
        })?;

        for x in 0..width_usize {
            let (red, green, blue, alpha) = match bit_count {
                1 => {
                    let byte = row[x / 8];
                    let index = usize::from((byte >> (7 - (x % 8))) & 1);
                    palette_pixel(palette, index, transparent_index, picture_type)?
                }
                4 => {
                    let byte = row[x / 2];
                    let index = usize::from(if x % 2 == 0 { byte >> 4 } else { byte & 0x0F });
                    palette_pixel(palette, index, transparent_index, picture_type)?
                }
                8 => {
                    let index = usize::from(row[x]);
                    palette_pixel(palette, index, transparent_index, picture_type)?
                }
                16 => {
                    let base = x * 2;
                    let value = u16::from_le_bytes([row[base], row[base + 1]]);
                    let red = u8::try_from(((value >> 10) & 0x1F) * 255 / 31).unwrap_or(255);
                    let green = u8::try_from(((value >> 5) & 0x1F) * 255 / 31).unwrap_or(255);
                    let blue = u8::try_from((value & 0x1F) * 255 / 31).unwrap_or(255);
                    (red, green, blue, 255)
                }
                24 => {
                    let base = x * 3;
                    (row[base + 2], row[base + 1], row[base], 255)
                }
                32 => {
                    let base = x * 4;
                    // WinHelp DIB records predate meaningful per-pixel alpha; treat the fourth byte
                    // as reserved rather than making old images unexpectedly translucent.
                    (row[base + 2], row[base + 1], row[base], 255)
                }
                _ => unreachable!("bit depth validated above"),
            };
            has_alpha |= alpha != 255;
            let out = (output_y * width_usize + x) * 4;
            rgba[out] = red;
            rgba[out + 1] = green;
            rgba[out + 2] = blue;
            rgba[out + 3] = alpha;
        }
    }

    Ok(DecodedPicture {
        width,
        height,
        rgba: Arc::from(rgba),
        has_alpha,
        sizing: PictureSizing::Pixels,
        hotspots: Arc::from([]),
    })
}

fn palette_pixel(
    palette: &[[u8; 3]],
    index: usize,
    transparent_index: Option<usize>,
    picture_type: u8,
) -> Result<(u8, u8, u8, u8), HlpError> {
    if palette.is_empty() && picture_type == 0x05 && index <= 1 {
        let value = if index == 0 { 0 } else { 255 };
        return Ok((value, value, value, 255));
    }
    let colour = palette.get(index).ok_or_else(|| {
        HlpError::invalid(
            "WinHelp bitmap palette",
            format!("pixel index {index} exceeds {} palette entries", palette.len()),
        )
    })?;
    let alpha = if transparent_index == Some(index) { 0 } else { 255 };
    Ok((colour[0], colour[1], colour[2], alpha))
}

fn read_compressed_unsigned_short(reader: &mut Reader<'_>) -> Result<u16, HlpError> {
    let first = reader.read_u8()?;
    if first & 1 == 0 {
        Ok(u16::from(first) / 2)
    } else {
        let second = reader.read_u8()?;
        Ok(u16::from(first) / 2 + u16::from(second) * 128)
    }
}

fn read_compressed_unsigned_long(reader: &mut Reader<'_>) -> Result<u32, HlpError> {
    let first = reader.read_u16()?;
    if first & 1 == 0 {
        Ok(u32::from(first) / 2)
    } else {
        let second = reader.read_u16()?;
        Ok(u32::from(first) / 2 + u32::from(second) * 32_768)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rle_supports_literal_and_repeated_runs() {
        let encoded = [0x83, 1, 2, 3, 0x03, 9];
        assert_eq!(rle_decompress(&encoded, 6).unwrap(), [1, 2, 3, 9, 9, 9]);
    }

    #[test]
    fn decodes_uncompressed_two_by_two_24bpp_dib() {
        let mut record = vec![0x06, 0x00];
        push_culong(&mut record, 0);
        push_culong(&mut record, 0);
        push_cushort(&mut record, 1);
        push_cushort(&mut record, 24);
        push_culong(&mut record, 2);
        push_culong(&mut record, 2);
        push_culong(&mut record, 0);
        push_culong(&mut record, 0);
        push_culong(&mut record, 16);
        push_culong(&mut record, 0);
        let offset_pos = record.len();
        record.extend_from_slice(&0_u32.to_le_bytes());
        record.extend_from_slice(&0_u32.to_le_bytes());
        let bits_offset = u32::try_from(record.len()).unwrap();
        record[offset_pos..offset_pos + 4].copy_from_slice(&bits_offset.to_le_bytes());
        // Bottom row: blue, white; top row: red, green. Each 2x24bpp row pads to 8 bytes.
        record.extend_from_slice(&[255, 0, 0, 255, 255, 255, 0, 0]);
        record.extend_from_slice(&[0, 0, 255, 0, 255, 0, 0, 0]);

        let mut stream = vec![0x34, 0x12, 1, 0];
        stream.extend_from_slice(&8_u32.to_le_bytes());
        stream.extend_from_slice(&record);
        let image = decode_graphics_stream(&stream).unwrap();
        assert_eq!((image.width, image.height), (2, 2));
        assert_eq!(&image.rgba[0..4], &[255, 0, 0, 255]);
        assert_eq!(&image.rgba[4..8], &[0, 255, 0, 255]);
        assert_eq!(&image.rgba[8..12], &[0, 0, 255, 255]);
        assert_eq!(&image.rgba[12..16], &[255, 255, 255, 255]);
    }

    #[test]
    fn decodes_indexed_dib_transparency() {
        let mut record = vec![0x06, 0x00];
        push_culong(&mut record, 0);
        push_culong(&mut record, 0);
        push_cushort(&mut record, 1);
        push_cushort(&mut record, 8);
        push_culong(&mut record, 1);
        push_culong(&mut record, 1);
        push_culong(&mut record, 2);
        push_culong(&mut record, 1);
        push_culong(&mut record, 4);
        push_culong(&mut record, 0);
        let offset_pos = record.len();
        record.extend_from_slice(&0_u32.to_le_bytes());
        record.extend_from_slice(&0_u32.to_le_bytes());
        // Palette entry 0 = red; entry 1 = green and, because clrImportant == 1, transparent.
        record.extend_from_slice(&[0, 0, 255, 0]);
        record.extend_from_slice(&[0, 255, 0, 0]);
        let bits_offset = u32::try_from(record.len()).unwrap();
        record[offset_pos..offset_pos + 4].copy_from_slice(&bits_offset.to_le_bytes());
        record.extend_from_slice(&[1, 0, 0, 0]);

        let mut stream = vec![0x34, 0x12, 1, 0];
        stream.extend_from_slice(&8_u32.to_le_bytes());
        stream.extend_from_slice(&record);
        let image = decode_graphics_stream(&stream).unwrap();
        assert_eq!((image.width, image.height), (1, 1));
        assert_eq!(&image.rgba[0..4], &[0, 255, 0, 0]);
        assert!(image.has_alpha);
    }


    #[test]
    fn parses_graphical_context_and_window_hotspots() {
        let mut hotspot_data = vec![0_u8, 2, 0, 0, 0, 0, 0];
        let mut first = [0_u8; 15];
        first[0] = 0xE7;
        first[3..5].copy_from_slice(&10_u16.to_le_bytes());
        first[5..7].copy_from_slice(&20_u16.to_le_bytes());
        first[7..9].copy_from_slice(&30_u16.to_le_bytes());
        first[9..11].copy_from_slice(&40_u16.to_le_bytes());
        hotspot_data.extend_from_slice(&first);
        let mut second = [0_u8; 15];
        second[0] = 0xEE;
        second[3..5].copy_from_slice(&50_u16.to_le_bytes());
        second[5..7].copy_from_slice(&60_u16.to_le_bytes());
        second[7..9].copy_from_slice(&70_u16.to_le_bytes());
        second[9..11].copy_from_slice(&80_u16.to_le_bytes());
        hotspot_data.extend_from_slice(&second);
        hotspot_data.extend_from_slice(b"first-name\0TOPIC_ONE\0second-name\0TOPIC_TWO>secondary\0");

        let mut record = vec![0_u8; 19];
        let offset = u32::try_from(record.len()).unwrap();
        let size = u32::try_from(hotspot_data.len()).unwrap();
        record.extend_from_slice(&hotspot_data);
        let hotspots = parse_graphical_hotspots(&record, size, offset).unwrap();
        assert_eq!(hotspots.len(), 2);
        assert_eq!((hotspots[0].x, hotspots[0].y, hotspots[0].width, hotspots[0].height), (10, 20, 30, 40));
        assert_eq!(
            hotspots[0].target,
            DecodedPictureHotspotTarget::Context {
                name: "TOPIC_ONE".to_owned(),
                popup: false,
                window_name: None,
                opcode: 0xE7,
            }
        );
        assert_eq!(
            hotspots[1].target,
            DecodedPictureHotspotTarget::Context {
                name: "TOPIC_TWO".to_owned(),
                popup: true,
                window_name: Some("secondary".to_owned()),
                opcode: 0xEE,
            }
        );
    }

    #[test]
    fn parses_graphical_macro_hotspot_without_executing_it() {
        let mut hotspot_data = vec![0_u8, 1, 0, 0, 0, 0, 0];
        let mut entry = [0_u8; 15];
        entry[0] = 0xC8;
        entry[3..5].copy_from_slice(&1_u16.to_le_bytes());
        entry[5..7].copy_from_slice(&2_u16.to_le_bytes());
        entry[7..9].copy_from_slice(&3_u16.to_le_bytes());
        entry[9..11].copy_from_slice(&4_u16.to_le_bytes());
        hotspot_data.extend_from_slice(&entry);
        hotspot_data.extend_from_slice(b"macro-name\0JumpId(`other.hlp', 42)\0");

        let size = u32::try_from(hotspot_data.len()).unwrap();
        let hotspots = parse_graphical_hotspots(&hotspot_data, size, 0).unwrap_or_default();
        // Offset zero means no hotspot table by definition; embed the same table after a prefix.
        assert!(hotspots.is_empty());
        let mut record = vec![0_u8; 4];
        let offset = u32::try_from(record.len()).unwrap();
        record.extend_from_slice(&hotspot_data);
        let hotspots = parse_graphical_hotspots(&record, size, offset).unwrap();
        assert_eq!(hotspots.len(), 1);
        assert_eq!(
            hotspots[0].target,
            DecodedPictureHotspotTarget::Macro("JumpId(`other.hlp', 42)".to_owned())
        );
    }

    #[test]
    fn metafile_mapping_modes_convert_authored_extents_to_pixels() {
        assert_eq!(metafile_pixel_dimensions(1, 320, 200, 120, 144), (320, 200));
        assert_eq!(metafile_pixel_dimensions(2, 254, 254, 120, 144), (120, 144));
        assert_eq!(metafile_pixel_dimensions(3, 2_540, 2_540, 120, 144), (120, 144));
        assert_eq!(metafile_pixel_dimensions(4, 100, 100, 120, 144), (120, 144));
        assert_eq!(metafile_pixel_dimensions(5, 1_000, 1_000, 120, 144), (120, 144));
        assert_eq!(metafile_pixel_dimensions(6, 1_440, 1_440, 120, 144), (120, 144));
        assert_eq!(metafile_pixel_dimensions(8, 2_540, 2_540, 120, 144), (120, 144));
    }

    #[test]
    fn bitmap_resolution_metadata_survives_decode() {
        let mut record = vec![0x06, 0x00];
        push_culong(&mut record, 120);
        push_culong(&mut record, 144);
        push_cushort(&mut record, 1);
        push_cushort(&mut record, 24);
        push_culong(&mut record, 2);
        push_culong(&mut record, 1);
        push_culong(&mut record, 0);
        push_culong(&mut record, 0);
        push_culong(&mut record, 8);
        push_culong(&mut record, 0);
        let offset_pos = record.len();
        record.extend_from_slice(&0_u32.to_le_bytes());
        record.extend_from_slice(&0_u32.to_le_bytes());
        let bits_offset = u32::try_from(record.len()).unwrap();
        record[offset_pos..offset_pos + 4].copy_from_slice(&bits_offset.to_le_bytes());
        record.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0]);

        let mut stream = vec![0x34, 0x12, 1, 0];
        stream.extend_from_slice(&8_u32.to_le_bytes());
        stream.extend_from_slice(&record);
        let image = decode_graphics_stream(&stream).unwrap();
        assert_eq!(
            image.sizing,
            PictureSizing::BitmapResolution {
                x_resolution: 120,
                y_resolution: 144,
            }
        );
    }

    fn push_cushort(target: &mut Vec<u8>, value: u16) {
        if value < 128 {
            target.push(u8::try_from(value * 2).unwrap());
        } else {
            let encoded = value * 2 + 1;
            target.extend_from_slice(&encoded.to_le_bytes());
        }
    }

    fn push_culong(target: &mut Vec<u8>, value: u32) {
        if value < 32_768 {
            let encoded = u16::try_from(value * 2).unwrap();
            target.extend_from_slice(&encoded.to_le_bytes());
        } else {
            let encoded = value * 2 + 1;
            target.extend_from_slice(&encoded.to_le_bytes());
        }
    }
}
