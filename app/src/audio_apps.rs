//! Event-driven discovery of render sessions; each application owns a native
//! process-tree loopback and retains its history independently of video capture.
use crate::audio::{AudioRecorder, AudioSnapshot};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

struct Entry {
    name: String,
    recorder: AudioRecorder,
    seen: Instant,
}
enum Message {
    Wake,
    Seconds(u32),
    Stop,
}
pub struct ApplicationAudio {
    entries: Arc<Mutex<BTreeMap<u32, Entry>>>,
    control: mpsc::Sender<Message>,
    worker: Option<std::thread::JoinHandle<()>>,
    pub events: mpsc::Receiver<Result<(), String>>,
}
impl ApplicationAudio {
    pub fn new(seconds: u32) -> Self {
        let entries = Arc::new(Mutex::new(BTreeMap::new()));
        let (control, rx) = mpsc::channel();
        let (tx, events) = mpsc::channel();
        let data = entries.clone();
        let wake = control.clone();
        let worker = std::thread::spawn(move || {
            #[cfg(windows)]
            {
                let mut seconds = seconds;
                loop {
                    match native::run(data.clone(), &rx, wake.clone(), tx.clone(), &mut seconds) {
                        Ok(()) => break,
                        Err(e) => {
                            let _ = tx.send(Err(format!("Дорожки приложений: {e:#}")));
                        }
                    }
                    match rx.recv_timeout(Duration::from_secs(2)) {
                        Ok(Message::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Ok(Message::Seconds(value)) => seconds = value,
                        _ => {}
                    }
                }
            }
        });
        Self {
            entries,
            control,
            worker: Some(worker),
            events,
        }
    }
    pub fn set_seconds(&self, seconds: u32) {
        let _ = self.control.send(Message::Seconds(seconds));
    }
    pub fn snapshots(&self, end: Instant, duration: f64) -> Vec<(String, AudioSnapshot)> {
        let mut entries = self.entries.lock().unwrap();
        #[cfg(windows)]
        {
            let Ok(tree) = native::processes() else {
                return Vec::new();
            };
            let blocked = native::steam_audio_roots(&tree);
            entries.retain(|pid, _| !blocked.contains(pid));
        }
        entries
            .values()
            .filter_map(|entry| {
                let snapshot = entry.recorder.snapshot(end, duration);
                snapshot
                    .is_audible()
                    .then(|| (entry.name.clone(), snapshot))
            })
            .collect()
    }
}
impl Drop for ApplicationAudio {
    fn drop(&mut self) {
        let _ = self.control.send(Message::Stop);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use anyhow::Result;
    use std::collections::BTreeSet;
    use windows::{
        Win32::{Media::Audio::*, System::Com::*},
        core::{Interface, implement},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_NO_MORE_FILES, GetLastError, INVALID_HANDLE_VALUE},
        System::Diagnostics::ToolHelp::*,
    };

    #[implement(IAudioSessionNotification)]
    struct Sessions {
        wake: mpsc::Sender<Message>,
    }
    impl IAudioSessionNotification_Impl for Sessions_Impl {
        fn OnSessionCreated(
            &self,
            _: windows::core::Ref<IAudioSessionControl>,
        ) -> windows::core::Result<()> {
            let _ = self.wake.send(Message::Wake);
            Ok(())
        }
    }
    pub(super) fn processes() -> std::io::Result<BTreeMap<u32, (u32, String)>> {
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

    pub(super) fn steam_audio_roots(processes: &BTreeMap<u32, (u32, String)>) -> BTreeSet<u32> {
        let mut blocked = BTreeSet::new();
        for (&pid, (_, name)) in processes {
            if ![
                "steam.exe",
                "steamwebhelper.exe",
                "steamservice.exe",
                "gameoverlayui.exe",
                "gameoverlayui64.exe",
            ]
            .iter()
            .any(|steam| name.eq_ignore_ascii_case(steam))
            {
                continue;
            }
            // WASAPI captures the whole tree below a selected PID. Reject its
            // ancestors too, while still allowing games in Steam's child trees.
            let mut current = pid;
            while current != 0 && blocked.insert(current) {
                let Some((parent, _)) = processes.get(&current) else {
                    break;
                };
                current = *parent;
            }
        }
        blocked
    }

    fn root(pid: u32, processes: &BTreeMap<u32, (u32, String)>) -> Option<(u32, String)> {
        let (_, name) = processes.get(&pid)?;
        let mut pid = pid;
        for _ in 0..32 {
            let (parent, _) = processes.get(&pid)?;
            if *parent == pid
                || !processes
                    .get(parent)
                    .is_some_and(|(_, n)| n.eq_ignore_ascii_case(name))
            {
                break;
            }
            pid = *parent;
        }
        let label = match name.to_lowercase().as_str() {
            "deadlock.exe" => "Deadlock",
            "discord.exe" => "Discord",
            "chrome.exe" => "Chrome",
            "msedge.exe" => "Edge",
            "firefox.exe" => "Firefox",
            "spotify.exe" => "Spotify",
            _ => name.trim_end_matches(".exe"),
        };
        Some((pid, label.to_owned()))
    }
    pub(super) fn run(
        entries: Arc<Mutex<BTreeMap<u32, Entry>>>,
        rx: &mpsc::Receiver<Message>,
        wake: mpsc::Sender<Message>,
        tx: mpsc::Sender<Result<(), String>>,
        seconds: &mut u32,
    ) -> Result<()> {
        unsafe {
            CoInitializeEx(None, COINIT_MULTITHREADED).ok()?;
            struct Com;
            impl Drop for Com {
                fn drop(&mut self) {
                    unsafe {
                        CoUninitialize();
                    }
                }
            }
            let _com = Com;
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL)?;
            let mut watchers: Vec<(String, IAudioSessionManager2, IAudioSessionNotification)> =
                Vec::new();
            let result = (|| -> Result<()> {
                loop {
                    let devices = enumerator.EnumAudioEndpoints(eRender, DEVICE_STATE_ACTIVE)?;
                    let mut ids = Vec::new();
                    for i in 0..devices.GetCount()? {
                        let device = devices.Item(i)?;
                        let id = device.GetId()?;
                        let key = id.to_string()?;
                        CoTaskMemFree(Some(id.0.cast()));
                        ids.push(key.clone());
                        if !watchers.iter().any(|(k, _, _)| k == &key) {
                            let manager: IAudioSessionManager2 =
                                device.Activate(CLSCTX_ALL, None)?;
                            let callback: IAudioSessionNotification =
                                Sessions { wake: wake.clone() }.into();
                            manager.RegisterSessionNotification(&callback)?;
                            watchers.push((key, manager, callback));
                        }
                    }
                    watchers.retain(|(id, manager, callback)| {
                        if ids.contains(id) {
                            true
                        } else {
                            let _ = manager.UnregisterSessionNotification(callback);
                            false
                        }
                    });
                    let now = Instant::now();
                    {
                        let mut map = entries.lock().unwrap();
                        let tree = processes()?;
                        let blocked = steam_audio_roots(&tree);
                        map.retain(|pid, _| !blocked.contains(pid));
                        for (_, manager, _) in &watchers {
                            let sessions = manager.GetSessionEnumerator()?;
                            for i in 0..sessions.GetCount()? {
                                let session = sessions.GetSession(i)?;
                                if session.GetState()? == AudioSessionStateExpired {
                                    continue;
                                }
                                let session: IAudioSessionControl2 = session.cast()?;
                                let pid = session.GetProcessId()?;
                                if pid == 0 {
                                    continue;
                                }
                                if let Some((pid, name)) = root(pid, &tree)
                                    && !blocked.contains(&pid)
                                {
                                    if let Some(entry) = map.get_mut(&pid) {
                                        entry.seen = now;
                                    } else {
                                        let recorder = AudioRecorder::for_process(pid)?;
                                        recorder.set_seconds(*seconds);
                                        map.insert(
                                            pid,
                                            Entry {
                                                name,
                                                recorder,
                                                seen: now,
                                            },
                                        );
                                    }
                                }
                            }
                        }
                        map.retain(|_, e| {
                            now.duration_since(e.seen) < Duration::from_secs(*seconds as u64 + 4)
                        });
                        for entry in map.values() {
                            while let Ok(status) = entry.recorder.events.try_recv() {
                                let _ = tx.send(status.map_err(|e| format!("{}: {e}", entry.name)));
                            }
                        }
                    }
                    match rx.recv_timeout(Duration::from_secs(1)) {
                        Ok(Message::Stop) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
                        Ok(Message::Seconds(value)) => {
                            *seconds = value;
                            for e in entries.lock().unwrap().values() {
                                e.recorder.set_seconds(*seconds);
                            }
                        }
                        _ => {}
                    }
                }
                Ok(())
            })();
            for (_, manager, callback) in watchers {
                let _ = manager.UnregisterSessionNotification(&callback);
            }
            if result.is_ok() {
                entries.lock().unwrap().clear();
            }
            result
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use crate::{audio::SAMPLE_RATE, media};
    struct Player(std::process::Child);
    impl Drop for Player {
        fn drop(&mut self) {
            media::stop_child(&mut self.0);
        }
    }
    #[test]
    #[ignore = "plays two quiet tones to verify application discovery, isolation and continuity"]
    fn discovers_simultaneous_apps_and_keeps_their_audio_separate_without_crackle() {
        let apps = ApplicationAudio::new(45);
        let play = |hz| {
            Player(
                media::command("ffplay")
                    .args([
                        "-nodisp",
                        "-autoexit",
                        "-f",
                        "lavfi",
                        "-i",
                        &format!("sine=frequency={hz}:sample_rate=48000:duration=10,volume=0.15"),
                    ])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .spawn()
                    .unwrap(),
            )
        };
        let first = play(600);
        let second = play(1200);
        let deadline = Instant::now() + Duration::from_secs(6);
        loop {
            if let Some(error) = apps.events.try_iter().find_map(Result::err) {
                panic!("{error}");
            }
            let entries = apps.entries.lock().unwrap();
            let ready = [first.0.id(), second.0.id()].iter().all(|pid| {
                entries
                    .get(pid)
                    .is_some_and(|e| e.recorder.snapshot(Instant::now(), 0.1).is_audible())
            });
            drop(entries);
            if ready {
                break;
            }
            assert!(
                Instant::now() < deadline,
                "application discovery did not capture both tones"
            );
            std::thread::sleep(Duration::from_millis(50));
        }
        std::thread::sleep(Duration::from_millis(1500));
        let entries = apps.entries.lock().unwrap();
        for (pid, hz, other) in [
            (first.0.id(), 600.0, 1200.0),
            (second.0.id(), 1200.0, 600.0),
        ] {
            let snapshot = entries[&pid]
                .recorder
                .snapshot(Instant::now() - Duration::from_millis(100), 1.0);
            let mut data = Vec::new();
            snapshot.write_to(&mut data).unwrap();
            let samples: Vec<f64> = data
                .chunks_exact(8)
                .map(|v| f32::from_le_bytes(v[..4].try_into().unwrap()) as f64)
                .collect();
            let component = |frequency: f64| {
                let (mut a, mut b) = (0.0, 0.0);
                for (i, s) in samples.iter().enumerate() {
                    let phase = std::f64::consts::TAU * frequency * i as f64 / SAMPLE_RATE as f64;
                    a += s * phase.sin();
                    b += s * phase.cos();
                }
                (
                    2.0 * a / samples.len() as f64,
                    2.0 * b / samples.len() as f64,
                )
            };
            let (a, b) = component(hz);
            let (x, y) = component(other);
            let signal = (a * a + b * b) / 2.0;
            let noise = samples
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    let phase = std::f64::consts::TAU * hz * i as f64 / SAMPLE_RATE as f64;
                    (s - a * phase.sin() - b * phase.cos()).powi(2)
                })
                .sum::<f64>()
                / samples.len() as f64;
            let snr = 10.0 * (signal / noise).log10();
            eprintln!(
                "PID={pid}, tone={hz}, SNR={snr:.1} dB, other-tone amplitude={:.7}",
                (x * x + y * y).sqrt()
            );
            assert!(signal > 1e-7, "no tone");
            assert!(x * x + y * y < signal * 0.0001, "other application leaked");
            assert!(
                snr > 30.0,
                "discontinuity/crackle in native PCM: {snr:.1} dB"
            );
        }
    }
}
