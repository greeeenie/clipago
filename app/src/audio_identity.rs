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
    use std::{
        collections::{BTreeMap, HashMap, HashSet},
        ffi::c_void,
        os::windows::ffi::OsStrExt,
        path::Path,
        ptr, slice,
        sync::Mutex,
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE},
        Graphics::Gdi::{
            BI_RGB, BITMAPINFO, BITMAPINFOHEADER, CreateCompatibleDC, CreateDIBSection,
            DIB_RGB_COLORS, DeleteDC, DeleteObject, SelectObject,
        },
        Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW},
        System::{
            Diagnostics::ToolHelp::{
                CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW,
                TH32CS_SNAPPROCESS,
            },
            Threading::{
                OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, QueryFullProcessImageNameW,
            },
        },
        UI::{
            Shell::ExtractIconExW,
            WindowsAndMessaging::{DI_NORMAL, DestroyIcon, DrawIconEx},
        },
    };

    pub fn processes() -> std::io::Result<BTreeMap<u32, (u32, String)>> {
        unsafe {
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            let mut entry = PROCESSENTRY32W {
                dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32,
                ..Default::default()
            };
            let mut result = BTreeMap::new();
            let mut ok = Process32FirstW(snapshot, &mut entry);
            while ok != 0 {
                let length = entry
                    .szExeFile
                    .iter()
                    .position(|v| *v == 0)
                    .unwrap_or(entry.szExeFile.len());
                let name = String::from_utf16_lossy(&entry.szExeFile[..length]);
                result.insert(entry.th32ProcessID, (entry.th32ParentProcessID, name));
                ok = Process32NextW(snapshot, &mut entry);
            }
            let status = GetLastError();
            CloseHandle(snapshot);
            if status != ERROR_NO_MORE_FILES {
                return Err(std::io::Error::from_raw_os_error(status as i32));
            }
            Ok(result)
        }
    }

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
        static WRITE_LOCK: Mutex<()> = Mutex::new(());
        let path = icon_path(name)?;
        if path.is_file() {
            return Ok(());
        }
        if let Some(bytes) = icon_png(executable) {
            let _write = WRITE_LOCK.lock().unwrap();
            if path.is_file() {
                return Ok(());
            }
            std::fs::create_dir_all(path.parent().unwrap())?;
            let temp = path.with_extension("tmp");
            std::fs::write(&temp, bytes)?;
            std::fs::rename(temp, path)?;
        }
        Ok(())
    }

    pub fn resolve_process_icons<'a>(
        sources: impl IntoIterator<Item = &'a str>,
    ) -> HashMap<String, String> {
        let mut missing: HashMap<String, Vec<&str>> = HashMap::new();
        for source in sources {
            missing
                .entry(source.to_lowercase())
                .or_default()
                .push(source);
        }
        let mut icons = HashMap::new();
        if missing.is_empty() {
            return icons;
        }
        let mut seen = HashSet::new();
        for (pid, (_, executable)) in processes().unwrap_or_default() {
            if missing.is_empty() {
                break;
            }
            let Some(path) = process_path(pid).filter(|path| seen.insert(path.clone())) else {
                continue;
            };
            let stem = Path::new(&executable)
                .file_stem()
                .unwrap_or_default()
                .to_string_lossy()
                .to_lowercase();
            let name = app_name(&executable, Some(&path)).to_lowercase();
            for key in [stem, name] {
                if let Some(sources) = missing.get(&key) {
                    let _ = cache_icon(sources[0], &path);
                    if let Some(icon) = icon_data(sources[0]) {
                        for source in sources {
                            icons.insert((*source).to_owned(), icon.clone());
                        }
                        missing.remove(&key);
                    }
                }
            }
        }
        icons
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
