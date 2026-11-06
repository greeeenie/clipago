//! Per-user Windows startup and one capture process per configuration profile.
use std::io;

pub fn background() -> bool {
    std::env::args_os().any(|arg| arg == "--background")
}

#[cfg(windows)]
mod platform {
    use super::*;
    use std::{
        hash::{DefaultHasher, Hash, Hasher},
        os::windows::ffi::OsStrExt,
        path::Path,
        ptr::{null, null_mut},
        sync::{
            Arc, Mutex,
            atomic::{AtomicBool, Ordering},
        },
    };
    use tauri::Manager;
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_ALREADY_EXISTS, GetLastError, HANDLE, WAIT_OBJECT_0},
        System::{
            Registry::{
                HKEY_CURRENT_USER, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ, RegCloseKey,
                RegCreateKeyExW, RegSetValueExW,
            },
            Threading::{CreateEventW, CreateMutexW, INFINITE, SetEvent, WaitForSingleObject},
        },
    };

    fn wide(value: impl AsRef<std::ffi::OsStr>) -> Vec<u16> {
        value.as_ref().encode_wide().chain(Some(0)).collect()
    }

    pub(super) fn command(exe: &Path) -> Vec<u16> {
        // Preserve non-ASCII paths and spaces without shell expansion.
        let mut value = vec![b'"' as u16];
        value.extend(exe.as_os_str().encode_wide());
        value.extend("\" --background\0".encode_utf16());
        value
    }

    fn write_entry(subkey: &str, exe: &Path) -> io::Result<()> {
        let mut key = null_mut();
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                wide(subkey).as_ptr(),
                0,
                null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                null(),
                &mut key,
                null_mut(),
            )
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        let data = command(exe);
        let status = unsafe {
            let status = RegSetValueExW(
                key,
                wide("Rewind").as_ptr(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * size_of::<u16>()) as u32,
            );
            RegCloseKey(key);
            status
        };
        if status != 0 {
            return Err(io::Error::from_raw_os_error(status as i32));
        }
        Ok(())
    }

    pub fn register() -> io::Result<()> {
        // Development and isolated QA must never replace the user's startup EXE.
        if cfg!(debug_assertions) || std::env::var_os("REWIND_CONFIG_DIR").is_some() {
            return Ok(());
        }
        write_entry(
            r"Software\Microsoft\Windows\CurrentVersion\Run",
            &std::env::current_exe()?,
        )
    }

    struct Handle(HANDLE);
    // Kernel handles are valid across threads; Arc keeps the event alive while waiting.
    unsafe impl Send for Handle {}
    unsafe impl Sync for Handle {}
    impl Drop for Handle {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    pub struct Instance {
        _mutex: Handle,
        show: Arc<Handle>,
        stopping: Arc<AtomicBool>,
        listener: Mutex<Option<std::thread::JoinHandle<()>>>,
    }

    impl Instance {
        pub fn acquire(background: bool) -> io::Result<Option<Self>> {
            let mut hash = DefaultHasher::new();
            rewind::settings::app_dirs()
                .map_err(io::Error::other)?
                .config_dir()
                .hash(&mut hash);
            if let Some(profile) = std::env::var_os("REWIND_CONFIG_DIR") {
                profile.hash(&mut hash);
            }
            let name = format!("Local\\Rewind.{:x}", hash.finish());
            Self::acquire_named(&name, background)
        }

        fn acquire_named(name: &str, background: bool) -> io::Result<Option<Self>> {
            let show = unsafe { CreateEventW(null(), 0, 0, wide(format!("{name}.show")).as_ptr()) };
            if show.is_null() {
                return Err(io::Error::last_os_error());
            }
            let show = Arc::new(Handle(show));
            let mutex = unsafe { CreateMutexW(null(), 0, wide(name).as_ptr()) };
            if mutex.is_null() {
                return Err(io::Error::last_os_error());
            }
            let already_exists = unsafe { GetLastError() } == ERROR_ALREADY_EXISTS;
            let mutex = Handle(mutex);
            if already_exists {
                if !background {
                    unsafe {
                        SetEvent(show.0);
                    }
                }
                return Ok(None);
            }
            Ok(Some(Self {
                _mutex: mutex,
                show,
                stopping: Arc::new(AtomicBool::new(false)),
                listener: Mutex::new(None),
            }))
        }

        pub fn listen(&self, app: tauri::AppHandle) {
            let show = self.show.clone();
            let stopping = self.stopping.clone();
            let listener = std::thread::spawn(move || {
                // A blocking kernel wait, with no timer or CPU polling.
                while unsafe { WaitForSingleObject(show.0, INFINITE) } == WAIT_OBJECT_0 {
                    if stopping.load(Ordering::Relaxed) {
                        break;
                    }
                    let handle = app.clone();
                    let _ = app.run_on_main_thread(move || {
                        if let Some(window) = handle.get_webview_window("main") {
                            let _ = window.show();
                            let _ = window.unminimize();
                            let _ = window.set_focus();
                        }
                    });
                }
            });
            *self.listener.lock().unwrap() = Some(listener);
        }

        pub fn shutdown(&self) {
            self.stopping.store(true, Ordering::Relaxed);
            unsafe {
                SetEvent(self.show.0);
            }
            if let Some(listener) = self.listener.lock().unwrap().take() {
                let _ = listener.join();
            }
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        #[test]
        fn startup_quotes_unicode_path_and_background_flag() {
            let value = command(Path::new(r"C:\Видео clips\Rewind.exe"));
            assert_eq!(
                String::from_utf16(&value[..value.len() - 1]).unwrap(),
                "\"C:\\Видео clips\\Rewind.exe\" --background"
            );
            assert_eq!(value.last(), Some(&0));
        }

        #[test]
        fn registry_round_trip_in_disposable_key() {
            use windows_sys::Win32::System::Registry::{
                RRF_RT_REG_SZ, RegDeleteTreeW, RegGetValueW,
            };
            let subkey = format!(r"Software\Rewind\Tests\Startup-{}", std::process::id());
            let exe = Path::new(r"C:\Видео clips\Rewind.exe");
            write_entry(&subkey, exe).unwrap();
            let mut actual = [0_u16; 256];
            let mut bytes = size_of_val(&actual) as u32;
            let status = unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    wide(&subkey).as_ptr(),
                    wide("Rewind").as_ptr(),
                    RRF_RT_REG_SZ,
                    null_mut(),
                    actual.as_mut_ptr().cast(),
                    &mut bytes,
                )
            };
            unsafe {
                RegDeleteTreeW(HKEY_CURRENT_USER, wide(&subkey).as_ptr());
            }
            assert_eq!(status, 0);
            assert_eq!(&actual[..bytes as usize / 2], command(exe));
        }

        #[test]
        fn duplicate_launch_only_signals_on_manual_open() {
            let name = format!("Local\\Rewind.test.{}", std::process::id());
            let first = Instance::acquire_named(&name, true).unwrap().unwrap();
            assert!(Instance::acquire_named(&name, true).unwrap().is_none());
            assert_ne!(
                unsafe { WaitForSingleObject(first.show.0, 0) },
                WAIT_OBJECT_0
            );
            assert!(Instance::acquire_named(&name, false).unwrap().is_none());
            assert_eq!(
                unsafe { WaitForSingleObject(first.show.0, 0) },
                WAIT_OBJECT_0
            );
            drop(first);
            assert!(Instance::acquire_named(&name, true).unwrap().is_some());
        }
    }
}

#[cfg(windows)]
pub use platform::{Instance, register};
