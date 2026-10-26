use std::{
    ffi::c_void,
    fs::{self, File},
    io::{BufWriter, Write},
    mem::size_of,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::UNIX_EPOCH,
};

use image::{DynamicImage, ImageFormat, RgbaImage};
use windows::{
    Win32::{
        Foundation::{RPC_E_CHANGED_MODE, SIZE},
        Graphics::Gdi::{
            BI_RGB, BITMAP, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, DIB_RGB_COLORS,
            DeleteDC, DeleteObject, GetDIBits, GetObjectW, HBITMAP, HDC, HGDIOBJ,
        },
        System::Com::{COINIT_APARTMENTTHREADED, CoInitializeEx, CoUninitialize},
        UI::Shell::{
            IShellItem, IShellItemImageFactory, SHCreateItemFromParsingName, SIIGBF_BIGGERSIZEOK,
            SIIGBF_ICONONLY,
        },
    },
    core::{HSTRING, Interface},
};

const FNV_OFFSET: u64 = 0xcbf29ce484222325;
const FNV_PRIME: u64 = 0x00000100000001b3;

/// Extract a 64px (or requested size) app icon and cache it as PNG.
/// Returns the PNG path on success. Must never panic.
pub fn ensure_app_icon(source: &Path, cache_dir: &Path, size: u32) -> Option<PathBuf> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        ensure_app_icon_inner(source, cache_dir, size)
    }))
    .ok()
    .flatten()
}

fn ensure_app_icon_inner(source: &Path, cache_dir: &Path, size: u32) -> Option<PathBuf> {
    let size = size.clamp(1, 1024);
    let cache_path = cache_file(source, cache_dir, size);

    if cache_path.is_file() {
        return Some(cache_path);
    }

    fs::create_dir_all(cache_dir).ok()?;

    if cache_path.is_file() {
        return Some(cache_path);
    }

    let (width, height, pixels) = extract_rgba(source, size)?;
    let rgba = RgbaImage::from_raw(width, height, pixels)?;
    let mut image = DynamicImage::ImageRgba8(rgba);

    if image.width() != size || image.height() != size {
        image = image.resize_exact(size, size, image::imageops::FilterType::Lanczos3);
    }

    write_png_atomic(&image, &cache_path)?;
    Some(cache_path)
}

fn cache_file(source: &Path, cache_dir: &Path, size: u32) -> PathBuf {
    let mut hash = FNV_OFFSET;

    hash_bytes(
        &mut hash,
        source.to_string_lossy().to_lowercase().as_bytes(),
    );
    hash_bytes(&mut hash, &[0]);
    hash_u64(&mut hash, u64::from(size));

    if let Ok(metadata) = fs::metadata(source) {
        hash_u64(&mut hash, metadata.len());

        if let Ok(modified) = metadata.modified()
            && let Ok(duration) = modified.duration_since(UNIX_EPOCH)
        {
            hash_u64(&mut hash, duration.as_secs());
            hash_u64(&mut hash, u64::from(duration.subsec_nanos()));
        }
    }

    cache_dir.join(format!("{hash:016x}.png"))
}

fn hash_bytes(hash: &mut u64, bytes: &[u8]) {
    for byte in bytes {
        *hash ^= u64::from(*byte);
        *hash = (*hash).wrapping_mul(FNV_PRIME);
    }
}

fn hash_u64(hash: &mut u64, value: u64) {
    hash_bytes(hash, &value.to_le_bytes());
}

fn extract_rgba(source: &Path, size: u32) -> Option<(u32, u32, Vec<u8>)> {
    let initialized = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    let _com = if initialized == RPC_E_CHANGED_MODE {
        None
    } else if initialized.is_ok() {
        Some(ComGuard)
    } else {
        return None;
    };

    let path = HSTRING::from(source.as_os_str());
    let item: IShellItem = unsafe { SHCreateItemFromParsingName(&path, None) }.ok()?;
    let factory: IShellItemImageFactory = item.cast().ok()?;
    let bitmap = unsafe {
        factory.GetImage(
            SIZE {
                cx: size as i32,
                cy: size as i32,
            },
            SIIGBF_ICONONLY | SIIGBF_BIGGERSIZEOK,
        )
    }
    .ok()?;

    if bitmap.is_invalid() {
        return None;
    }
    let bitmap = OwnedBitmap(bitmap);

    let mut gdi_bitmap = BITMAP::default();
    let copied = unsafe {
        GetObjectW(
            HGDIOBJ::from(bitmap.0),
            size_of::<BITMAP>() as i32,
            Some(&mut gdi_bitmap as *mut BITMAP as *mut c_void),
        )
    };

    if copied == 0 || gdi_bitmap.bmWidth <= 0 || gdi_bitmap.bmHeight <= 0 {
        return None;
    }
    if gdi_bitmap.bmWidth > 8192 || gdi_bitmap.bmHeight > 8192 {
        return None;
    }

    let width = gdi_bitmap.bmWidth;
    let height = gdi_bitmap.bmHeight;
    let byte_len = (width as usize)
        .checked_mul(height as usize)?
        .checked_mul(4)?;

    let mut info = BITMAPINFO::default();
    info.bmiHeader.biSize = size_of::<BITMAPINFOHEADER>() as u32;
    info.bmiHeader.biWidth = width;
    info.bmiHeader.biHeight = -height;
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB.0;
    info.bmiHeader.biSizeImage = (width as u32)
        .saturating_mul(height as u32)
        .saturating_mul(4);

    let hdc = unsafe { CreateCompatibleDC(None) };
    if hdc.is_invalid() {
        return None;
    }
    let hdc = OwnedDc(hdc);

    let mut pixels = vec![0u8; byte_len];
    let scanlines = unsafe {
        GetDIBits(
            hdc.0,
            bitmap.0,
            0,
            height as u32,
            Some(pixels.as_mut_ptr() as *mut c_void),
            &mut info,
            DIB_RGB_COLORS,
        )
    };

    if scanlines != height {
        return None;
    }

    let mut has_alpha = false;
    for pixel in pixels.chunks_exact_mut(4) {
        pixel.swap(0, 2);
        has_alpha |= pixel[3] != 0;
    }

    if !has_alpha {
        for pixel in pixels.chunks_exact_mut(4) {
            pixel[3] = 255;
        }
    }

    Some((width as u32, height as u32, pixels))
}

fn write_png_atomic(image: &DynamicImage, path: &Path) -> Option<()> {
    static TEMP_SEQUENCE: AtomicU64 = AtomicU64::new(0);

    let file_name = path.file_name()?.to_string_lossy();
    let sequence = TEMP_SEQUENCE.fetch_add(1, Ordering::Relaxed);
    let temp = path.with_file_name(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        sequence
    ));

    let written = (|| -> Option<()> {
        let file = File::create(&temp).ok()?;
        let mut writer = BufWriter::new(file);
        image.write_to(&mut writer, ImageFormat::Png).ok()?;
        writer.flush().ok()?;
        Some(())
    })();

    if written.is_none() {
        let _ = fs::remove_file(&temp);
        return None;
    }

    match fs::rename(&temp, path) {
        Ok(()) => Some(()),
        Err(_) => {
            let _ = fs::remove_file(&temp);
            if path.is_file() { Some(()) } else { None }
        }
    }
}

struct ComGuard;

impl Drop for ComGuard {
    fn drop(&mut self) {
        unsafe {
            CoUninitialize();
        }
    }
}

struct OwnedBitmap(HBITMAP);

impl Drop for OwnedBitmap {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteObject(HGDIOBJ::from(self.0));
        }
    }
}

struct OwnedDc(HDC);

impl Drop for OwnedDc {
    fn drop(&mut self) {
        unsafe {
            let _ = DeleteDC(self.0);
        }
    }
}
