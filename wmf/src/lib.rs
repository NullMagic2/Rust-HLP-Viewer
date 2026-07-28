//! Narrow platform adapter for rendering legacy Windows metafiles.
//!
//! Classic WinHelp stores 16-bit Windows-format WMF payloads. The portable HLP parser keeps
//! `unsafe_code = deny`; this small crate is the only place that crosses into GDI so Windows can
//! convert and play those historical drawing records into a 32-bit top-down DIB.

/// Fully rendered opaque RGBA pixels returned to the GUI-independent HLP engine.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RgbaImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

/// Converts a legacy Windows-format metafile into top-down RGBA pixels.
///
/// On non-Windows hosts there is intentionally no fake software implementation: callers receive a
/// descriptive error and retain their normal unsupported-picture placeholder.
pub fn render_windows_metafile(
    bytes: &[u8],
    width: u32,
    height: u32,
    mapping_mode: i32,
    x_extent: i32,
    y_extent: i32,
) -> Result<RgbaImage, String> {
    if bytes.is_empty() {
        return Err("Windows metafile payload is empty".to_owned());
    }
    if width == 0 || height == 0 {
        return Err(format!("Windows metafile has invalid dimensions {width}x{height}"));
    }
    #[cfg(windows)]
    {
        windows_impl::render(bytes, width, height, mapping_mode, x_extent, y_extent)
    }
    #[cfg(not(windows))]
    {
        let _ = (mapping_mode, x_extent, y_extent);
        Err("Windows metafile rendering is available only on Windows".to_owned())
    }
}

#[cfg(windows)]
mod windows_impl {
    use super::RgbaImage;
    use std::ffi::c_void;
    use std::mem::size_of;
    use std::ptr::null_mut;
    use std::slice;
    use windows_sys::Win32::Foundation::RECT;
    use windows_sys::Win32::Graphics::Gdi::{
        BI_RGB, BITMAPINFO, CreateCompatibleDC, CreateDIBSection, DIB_RGB_COLORS,
        DeleteDC, DeleteEnhMetaFile, DeleteObject, GdiFlush, PlayEnhMetaFile, SelectObject,
    };
    use windows_sys::Win32::System::DataExchange::{METAFILEPICT, SetWinMetaFileBits};

    /// Renders through a memory DC backed by a top-down 32-bit DIB section.
    pub(super) fn render(
        bytes: &[u8],
        width: u32,
        height: u32,
        mapping_mode: i32,
        x_extent: i32,
        y_extent: i32,
    ) -> Result<RgbaImage, String> {
        let width_i32 = i32::try_from(width).map_err(|_| "WMF width does not fit i32")?;
        let height_i32 = i32::try_from(height).map_err(|_| "WMF height does not fit i32")?;
        let byte_count = usize::try_from(width)
            .ok()
            .and_then(|w| usize::try_from(height).ok().and_then(|h| w.checked_mul(h)))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| "WMF RGBA buffer size overflow".to_owned())?;
        let input_len = u32::try_from(bytes.len())
            .map_err(|_| "WMF payload is too large for the Windows metafile API".to_owned())?;

        // SAFETY: all GDI handles created in this block are checked for null and released before
        // returning. The DIB memory pointer is used only for its documented width*height*4 extent,
        // after GdiFlush has synchronized GDI drawing into the section.
        unsafe {
            let memory_dc = CreateCompatibleDC(null_mut());
            if memory_dc.is_null() {
                return Err("CreateCompatibleDC failed while rendering WMF".to_owned());
            }

            let mut bitmap_info = BITMAPINFO::default();
            bitmap_info.bmiHeader.biSize = u32::try_from(size_of::<windows_sys::Win32::Graphics::Gdi::BITMAPINFOHEADER>())
                .unwrap_or(40);
            bitmap_info.bmiHeader.biWidth = width_i32;
            // Negative height makes the section top-down, matching DecodedPicture.
            bitmap_info.bmiHeader.biHeight = -height_i32;
            bitmap_info.bmiHeader.biPlanes = 1;
            bitmap_info.bmiHeader.biBitCount = 32;
            bitmap_info.bmiHeader.biCompression = BI_RGB;
            bitmap_info.bmiHeader.biSizeImage = u32::try_from(byte_count).unwrap_or(u32::MAX);

            let mut bits: *mut c_void = null_mut();
            let bitmap = CreateDIBSection(
                memory_dc,
                &bitmap_info,
                DIB_RGB_COLORS,
                &mut bits,
                null_mut(),
                0,
            );
            if bitmap.is_null() || bits.is_null() {
                let _ = DeleteDC(memory_dc);
                return Err("CreateDIBSection failed while rendering WMF".to_owned());
            }

            let old_bitmap = SelectObject(memory_dc, bitmap.cast());
            if old_bitmap.is_null() {
                let _ = DeleteObject(bitmap.cast());
                let _ = DeleteDC(memory_dc);
                return Err("SelectObject failed while preparing WMF render target".to_owned());
            }

            let dib = slice::from_raw_parts_mut(bits.cast::<u8>(), byte_count);
            // WinHelp topic backgrounds are opaque; initializing to white mirrors the historical
            // viewer and avoids inventing alpha semantics that Windows-format metafiles never had.
            for pixel in dib.chunks_exact_mut(4) {
                pixel.copy_from_slice(&[255, 255, 255, 255]);
            }

            let metafile_pict = METAFILEPICT {
                mm: mapping_mode,
                xExt: x_extent,
                yExt: y_extent,
                hMF: null_mut(),
            };
            let enhanced = SetWinMetaFileBits(
                input_len,
                bytes.as_ptr(),
                memory_dc,
                &metafile_pict,
            );
            if enhanced.is_null() {
                let _ = SelectObject(memory_dc, old_bitmap);
                let _ = DeleteObject(bitmap.cast());
                let _ = DeleteDC(memory_dc);
                return Err("SetWinMetaFileBits could not convert the WinHelp WMF".to_owned());
            }

            let target = RECT {
                left: 0,
                top: 0,
                right: width_i32,
                bottom: height_i32,
            };
            let played = PlayEnhMetaFile(memory_dc, enhanced, &target);
            let _ = GdiFlush();

            let mut rgba = vec![0_u8; byte_count];
            if played != 0 {
                for (source, target) in dib.chunks_exact(4).zip(rgba.chunks_exact_mut(4)) {
                    // DIB sections are BGRA; GDI's alpha byte is not meaningful for legacy WMF.
                    target.copy_from_slice(&[source[2], source[1], source[0], 255]);
                }
            }

            let _ = DeleteEnhMetaFile(enhanced);
            let _ = SelectObject(memory_dc, old_bitmap);
            let _ = DeleteObject(bitmap.cast());
            let _ = DeleteDC(memory_dc);

            if played == 0 {
                return Err("PlayEnhMetaFile failed while rendering the WinHelp WMF".to_owned());
            }
            Ok(RgbaImage { width, height, rgba })
        }
    }
}
