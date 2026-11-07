use std::{io::Read, path::PathBuf};

use base64::{Engine as _, engine::general_purpose::STANDARD};
use sha2::{Digest, Sha256};

fn icon_path(name: &str) -> anyhow::Result<PathBuf> {
    let directory = crate::settings::data_file("audio-icons")?;
    let digest = Sha256::digest(name.to_lowercase().as_bytes());
    Ok(directory.join(format!("{digest:x}.png")))
}

pub fn icon_data(name: &str) -> Option<String> {
    let mut bytes = Vec::new();
    std::fs::File::open(icon_path(name).ok()?)
        .ok()?
        .take(8193)
        .read_to_end(&mut bytes)
        .ok()?;
    (bytes.len() <= 8192 && bytes.starts_with(b"\x89PNG\r\n\x1a\n"))
        .then(|| format!("data:image/png;base64,{}", STANDARD.encode(bytes)))
}

#[cfg(windows)]
pub mod windows {
    use super::*;
    use std::{ffi::c_void, os::windows::ffi::OsStrExt, path::Path, ptr, slice};
    use windows_sys::Win32::{
        Foundation::CloseHandle,
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, SelectObject,
        },
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::Threading::{
            OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
        },
        UI::{
            Shell::ExtractIconExW,
            WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx},
        },
    };

    pub fn process_path(pid: u32) -> Option<PathBuf> {
        unsafe {
            let process = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
            if process.is_null() {
                return None;
            }
            let mut path = vec![0_u16; 32768];
            let mut len = path.len() as u32;
            let ok = QueryFullProcessImageNameW(process, 0, path.as_mut_ptr(), &mut len);
            CloseHandle(process);
            (ok != 0).then(|| {
                use std::os::windows::ffi::OsStringExt;
                PathBuf::from(std::ffi::OsString::from_wide(&path[..len as usize]))
            })
        }
    }

    fn wide_path(path: &Path) -> Vec<u16> {
        path.as_os_str().encode_wide().chain([0]).collect()
    }

    pub fn app_name(executable: &str, path: Option<&Path>) -> String {
        if let Some(name) = path.and_then(version_name) {
            return name;
        }
        let stem = Path::new(executable)
            .file_stem()
            .and_then(|stem| stem.to_str())
            .unwrap_or(executable)
            .replace(['_', '-'], " ");
        let mut chars = stem.chars();
        chars.next().map_or(stem.clone(), |first| {
            first.to_uppercase().chain(chars).collect()
        })
    }

    fn version_name(path: &Path) -> Option<String> {
        let path = wide_path(path);
        unsafe {
            let len = GetFileVersionInfoSizeW(path.as_ptr(), ptr::null_mut());
            if len == 0 || len > 1024 * 1024 {
                return None;
            }
            let mut data = vec![0_u32; (len as usize).div_ceil(4)];
            if GetFileVersionInfoW(path.as_ptr(), 0, len, data.as_mut_ptr().cast()) == 0 {
                return None;
            }
            let query = |key: &str| {
                let key: Vec<_> = key.encode_utf16().chain([0]).collect();
                let mut value = ptr::null_mut::<c_void>();
                let mut count = 0;
                let ok = VerQueryValueW(data.as_ptr().cast(), key.as_ptr(), &mut value, &mut count);
                (ok != 0 && !value.is_null() && count != 0).then_some((value, count as usize))
            };
            let (table, count) = query(r"\VarFileInfo\Translation")?;
            let pairs = slice::from_raw_parts(table.cast::<u16>(), count / 2);
            for pair in pairs.chunks_exact(2) {
                for field in ["FileDescription", "ProductName"] {
                    let key = format!(r"\StringFileInfo\{:04x}{:04x}\{field}", pair[0], pair[1]);
                    if let Some((value, count)) = query(&key) {
                        let units = slice::from_raw_parts(value.cast::<u16>(), count);
                        let value = String::from_utf16_lossy(units);
                        let name = value.trim_end_matches('\0').trim();
                        let generic = name.to_lowercase().contains("operating system");
                        if !name.is_empty()
                            && !generic
                            && name.chars().count() <= 80
                            && !name.chars().any(char::is_control)
                        {
                            return Some(name.to_owned());
                        }
                    }
                }
            }
        }
        None
    }

    pub fn cache_icon(name: &str, executable: &Path) -> anyhow::Result<()> {
        let path = icon_path(name)?;
        if path.is_file() {
            return Ok(());
        }
        if let Some(bytes) = icon_png(executable) {
            std::fs::create_dir_all(path.parent().unwrap())?;
            let temp = path.with_extension("tmp");
            std::fs::write(&temp, bytes)?;
            std::fs::rename(temp, path)?;
        }
        Ok(())
    }

    fn icon_png(path: &Path) -> Option<Vec<u8>> {
        const SIZE: i32 = 32;
        let path = wide_path(path);
        let mut icon = ptr::null_mut();
        let pixels = unsafe {
            if ExtractIconExW(path.as_ptr(), 0, &mut icon, ptr::null_mut(), 1) == 0
                || icon.is_null()
            {
                return None;
            }
            let dc = CreateCompatibleDC(ptr::null_mut());
            if dc.is_null() {
                DestroyIcon(icon);
                return None;
            }
            let mut bitmap: BITMAPINFO = std::mem::zeroed();
            bitmap.bmiHeader = BITMAPINFOHEADER {
                biSize: std::mem::size_of::<BITMAPINFOHEADER>() as u32,
                biWidth: SIZE,
                biHeight: -SIZE,
                biPlanes: 1,
                biBitCount: 32,
                biCompression: BI_RGB,
                ..std::mem::zeroed()
            };
            let mut pixels = ptr::null_mut();
            let dib =
                CreateDIBSection(dc, &bitmap, DIB_RGB_COLORS, &mut pixels, ptr::null_mut(), 0);
            let rgba = if !dib.is_null() && !pixels.is_null() {
                let bytes =
                    slice::from_raw_parts_mut(pixels.cast::<u8>(), (SIZE * SIZE * 4) as usize);
                bytes.fill(0);
                let old = SelectObject(dc, dib);
                let drawn = !old.is_null()
                    && DrawIconEx(dc, 0, 0, icon, SIZE, SIZE, 0, ptr::null_mut(), DI_NORMAL) != 0;
                if !old.is_null() {
                    SelectObject(dc, old);
                }
                drawn.then(|| {
                    for pixel in bytes.chunks_exact_mut(4) {
                        pixel.swap(0, 2);
                        let alpha = u16::from(pixel[3]);
                        if alpha != 0 {
                            for channel in &mut pixel[..3] {
                                *channel = (u16::from(*channel) * 255 / alpha).min(255) as u8;
                            }
                        }
                    }
                    bytes.to_vec()
                })
            } else {
                None
            };
            if !dib.is_null() {
                DeleteObject(dib);
            }
            DeleteDC(dc);
            DestroyIcon(icon);
            rgba?
        };
        if !pixels.chunks_exact(4).any(|pixel| pixel[3] != 0) {
            return None;
        }
        let mut png = Vec::new();
        let mut encoder = png::Encoder::new(&mut png, SIZE as u32, SIZE as u32);
        encoder.set_color(png::ColorType::Rgba);
        encoder
            .write_header()
            .ok()?
            .write_image_data(&pixels)
            .ok()?;
        Some(png)
    }
}
