//! Event-driven WASAPI render-loopback. Audio survives video-source restarts.
//! A bounded PCM ring avoids another always-running encoder process.
use anyhow::Result;
use std::{
    collections::VecDeque,
    io::Write,
    sync::{Arc, Mutex, mpsc},
    time::{Duration, Instant},
};

pub const SAMPLE_RATE: usize = 48_000;
const FRAME_BYTES: usize = 8; // stereo IEEE float
const KEEP_SECONDS: u64 = 124;
const BYTE_LIMIT: usize = 50 * 1024 * 1024;

#[derive(Clone)]
struct Chunk {
    start: Instant,
    data: Arc<[u8]>,
}
struct Ring {
    chunks: VecDeque<Chunk>,
    bytes: usize,
    keep_seconds: u64,
}
impl Default for Ring {
    fn default() -> Self {
        Self {
            chunks: VecDeque::new(),
            bytes: 0,
            keep_seconds: KEEP_SECONDS,
        }
    }
}
impl Ring {
    fn push(&mut self, start: Instant, data: Vec<u8>) {
        // Silent applications need no PCM storage; snapshots fill these gaps.
        if data.iter().any(|v| *v != 0) {
            self.bytes += data.len();
            self.chunks.push_back(Chunk {
                start,
                data: data.into(),
            });
        }
        while self.chunks.front().is_some_and(|first| {
            start.saturating_duration_since(first.start) > Duration::from_secs(self.keep_seconds)
        }) || self.bytes > BYTE_LIMIT
        {
            if let Some(chunk) = self.chunks.pop_front() {
                self.bytes -= chunk.data.len();
            }
        }
    }
}

#[derive(Default)]
struct PacketClock {
    anchor: Option<Instant>,
    frames: u64,
    next_device: Option<u64>,
}
impl PacketClock {
    fn packet(&mut self, stamp: Instant, device: u64, frames: u32, discontinuity: bool) -> Instant {
        // QPC is an initial wall-clock anchor, not a new sample offset per packet.
        // Re-anchoring every 10 ms inserts/drops samples due to scheduler jitter.
        if self.anchor.is_none() || (discontinuity && self.next_device != Some(device)) {
            self.anchor = Some(stamp);
            self.frames = 0;
        }
        let start =
            self.anchor.unwrap() + Duration::from_secs_f64(self.frames as f64 / SAMPLE_RATE as f64);
        self.frames += frames as u64;
        self.next_device = Some(device + frames as u64);
        start
    }
}

#[derive(Clone)]
pub struct AudioSnapshot {
    chunks: Vec<Chunk>,
    start: Instant,
    frames: usize,
}
impl AudioSnapshot {
    pub fn is_audible(&self) -> bool {
        !self.chunks.is_empty()
    }
    /// Align to the video window; unavailable output/device-transition gaps are silence.
    pub fn write_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
        let mut position = 0;
        let zeros = [0_u8; 8192];
        for chunk in &self.chunks {
            let offset = if chunk.start >= self.start {
                (chunk.start.duration_since(self.start).as_secs_f64() * SAMPLE_RATE as f64).round()
                    as i64
            } else {
                -((self.start.duration_since(chunk.start).as_secs_f64() * SAMPLE_RATE as f64)
                    .round() as i64)
            };
            let count = chunk.data.len() / FRAME_BYTES;
            let from = (position as i64 - offset).max(0) as usize;
            let to = count.min((self.frames as i64 - offset).max(0) as usize);
            if from >= to {
                continue;
            }
            let target = (offset + from as i64).max(0) as usize;
            let mut gap = target.saturating_sub(position) * FRAME_BYTES;
            while gap > 0 {
                let n = gap.min(zeros.len());
                writer.write_all(&zeros[..n])?;
                gap -= n;
            }
            writer.write_all(&chunk.data[from * FRAME_BYTES..to * FRAME_BYTES])?;
            position = (offset + to as i64).max(0) as usize;
        }
        let mut gap = self.frames.saturating_sub(position) * FRAME_BYTES;
        while gap > 0 {
            let n = gap.min(zeros.len());
            writer.write_all(&zeros[..n])?;
            gap -= n;
        }
        Ok(())
    }
}

pub struct AudioRecorder {
    ring: Arc<Mutex<Ring>>,
    pub events: mpsc::Receiver<std::result::Result<(), String>>,
    #[cfg(windows)]
    stop: Arc<native::Event>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl AudioRecorder {
    pub fn new() -> Result<Self> {
        Self::target(None)
    }
    pub fn for_process(pid: u32) -> Result<Self> {
        Self::target(Some(pid))
    }
    fn target(pid: Option<u32>) -> Result<Self> {
        let ring = Arc::new(Mutex::new(Ring::default()));
        let (tx, events) = mpsc::channel();
        #[cfg(windows)]
        {
            let stop = Arc::new(native::Event::new()?);
            let flag = stop.clone();
            let data = ring.clone();
            let worker = std::thread::spawn(move || native::run(data, flag, tx, pid));
            Ok(Self {
                ring,
                events,
                stop,
                worker: Some(worker),
            })
        }
        #[cfg(not(windows))]
        {
            let _ = tx;
            anyhow::bail!("WASAPI доступен только в Windows");
        }
    }
    pub fn snapshot(&self, end: Instant, duration: f64) -> AudioSnapshot {
        let start = end - Duration::from_secs_f64(duration);
        let chunks = self
            .ring
            .lock()
            .unwrap()
            .chunks
            .iter()
            .filter(|chunk| {
                chunk.start < end
                    && chunk.start
                        + Duration::from_secs_f64(
                            chunk.data.len() as f64 / (FRAME_BYTES * SAMPLE_RATE) as f64,
                        )
                        > start
            })
            .cloned()
            .collect();
        AudioSnapshot {
            chunks,
            start,
            frames: (duration * SAMPLE_RATE as f64).round() as usize,
        }
    }
    pub fn set_seconds(&self, seconds: u32) {
        self.ring.lock().unwrap().keep_seconds = seconds.clamp(1, 120) as u64 + 4;
    }
}
impl Drop for AudioRecorder {
    fn drop(&mut self) {
        #[cfg(windows)]
        self.stop.signal();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(windows)]
mod native {
    use super::*;
    use anyhow::Context;
    use windows::{
        Win32::{Media::Audio::*, System::Com::*},
        core::{Interface, PCWSTR, implement},
    };
    use windows_sys::Win32::{
        Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0},
        System::{
            Performance::{QueryPerformanceCounter, QueryPerformanceFrequency},
            Threading::{
                CreateEventW, INFINITE, SetEvent, WaitForMultipleObjects, WaitForSingleObject,
            },
        },
    };

    pub(super) struct Event(HANDLE);
    unsafe impl Send for Event {}
    unsafe impl Sync for Event {}
    impl Event {
        pub fn new() -> Result<Self> {
            let handle = unsafe { CreateEventW(std::ptr::null(), 0, 0, std::ptr::null()) };
            if handle.is_null() {
                return Err(std::io::Error::last_os_error().into());
            }
            Ok(Self(handle))
        }
        pub fn signal(&self) {
            unsafe {
                SetEvent(self.0);
            }
        }
    }
    impl Drop for Event {
        fn drop(&mut self) {
            unsafe {
                CloseHandle(self.0);
            }
        }
    }

    #[implement(IMMNotificationClient)]
    struct Notifications {
        changed: Arc<Event>,
    }
    impl IMMNotificationClient_Impl for Notifications_Impl {
        fn OnDefaultDeviceChanged(
            &self,
            flow: EDataFlow,
            role: ERole,
            _: &PCWSTR,
        ) -> windows::core::Result<()> {
            if flow == eRender && role == eMultimedia {
                self.changed.signal();
            }
            Ok(())
        }
        fn OnDeviceStateChanged(&self, _: &PCWSTR, _: DEVICE_STATE) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnDeviceAdded(&self, _: &PCWSTR) -> windows::core::Result<()> {
            Ok(())
        }
        fn OnDeviceRemoved(&self, _: &PCWSTR) -> windows::core::Result<()> {
            self.changed.signal();
            Ok(())
        }
        fn OnPropertyValueChanged(
            &self,
            _: &PCWSTR,
            _: &windows::Win32::Foundation::PROPERTYKEY,
        ) -> windows::core::Result<()> {
            Ok(())
        }
    }

    #[implement(IActivateAudioInterfaceCompletionHandler)]
    struct Activation {
        done: Arc<Event>,
    }
    impl IActivateAudioInterfaceCompletionHandler_Impl for Activation_Impl {
        fn ActivateCompleted(
            &self,
            _: windows::core::Ref<IActivateAudioInterfaceAsyncOperation>,
        ) -> windows::core::Result<()> {
            self.done.signal();
            Ok(())
        }
    }
    fn process_client(pid: u32, stop: &Event) -> Result<IAudioClient> {
        use windows::Win32::System::{Com::StructuredStorage::*, Variant::VT_BLOB};
        unsafe {
            let mut params = AUDIOCLIENT_ACTIVATION_PARAMS {
                ActivationType: AUDIOCLIENT_ACTIVATION_TYPE_PROCESS_LOOPBACK,
                Anonymous: AUDIOCLIENT_ACTIVATION_PARAMS_0 {
                    ProcessLoopbackParams: AUDIOCLIENT_PROCESS_LOOPBACK_PARAMS {
                        TargetProcessId: pid,
                        ProcessLoopbackMode: PROCESS_LOOPBACK_MODE_INCLUDE_TARGET_PROCESS_TREE,
                    },
                },
            };
            // The blob borrows stack parameters. Do not PropVariantClear it.
            let variant = std::mem::ManuallyDrop::new(PROPVARIANT {
                Anonymous: PROPVARIANT_0 {
                    Anonymous: std::mem::ManuallyDrop::new(PROPVARIANT_0_0 {
                        vt: VT_BLOB,
                        Anonymous: PROPVARIANT_0_0_0 {
                            blob: BLOB {
                                cbSize: std::mem::size_of_val(&params) as u32,
                                pBlobData: (&mut params as *mut AUDIOCLIENT_ACTIVATION_PARAMS)
                                    .cast(),
                            },
                        },
                        ..Default::default()
                    }),
                },
            });
            let done = Arc::new(Event::new()?);
            let handler: IActivateAudioInterfaceCompletionHandler =
                Activation { done: done.clone() }.into();
            let operation = ActivateAudioInterfaceAsync(
                VIRTUAL_AUDIO_DEVICE_PROCESS_LOOPBACK,
                &IAudioClient::IID,
                Some(&*variant),
                &handler,
            )?;
            let handles = [stop.0, done.0];
            anyhow::ensure!(
                WaitForMultipleObjects(2, handles.as_ptr(), 0, 5000) == WAIT_OBJECT_0 + 1,
                "Захват приложения отменён или не ответил"
            );
            let mut status = windows::core::HRESULT(0);
            let mut object = None;
            operation.GetActivateResult(&mut status, &mut object)?;
            status.ok()?;
            Ok(object.context("Нет аудиоклиента приложения")?.cast()?)
        }
    }

    fn capture(
        enumerator: &IMMDeviceEnumerator,
        ring: &Mutex<Ring>,
        stop: &Event,
        changed: &Event,
        tx: &mpsc::Sender<std::result::Result<(), String>>,
        pid: Option<u32>,
    ) -> Result<bool> {
        unsafe {
            let client: IAudioClient = if let Some(pid) = pid {
                process_client(pid, stop)?
            } else {
                enumerator
                    .GetDefaultAudioEndpoint(eRender, eMultimedia)?
                    .Activate(CLSCTX_ALL, None)?
            };
            let format = WAVEFORMATEX {
                wFormatTag: 3,
                nChannels: 2,
                nSamplesPerSec: SAMPLE_RATE as u32,
                nAvgBytesPerSec: (SAMPLE_RATE * FRAME_BYTES) as u32,
                nBlockAlign: FRAME_BYTES as u16,
                wBitsPerSample: 32,
                cbSize: 0,
            };
            client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                AUDCLNT_STREAMFLAGS_LOOPBACK
                    | AUDCLNT_STREAMFLAGS_EVENTCALLBACK
                    | AUDCLNT_STREAMFLAGS_AUTOCONVERTPCM
                    | AUDCLNT_STREAMFLAGS_SRC_DEFAULT_QUALITY,
                0,
                0,
                &format,
                None,
            )?;
            let ready = Event::new()?;
            client.SetEventHandle(windows::Win32::Foundation::HANDLE(ready.0))?;
            let capture: IAudioCaptureClient = client.GetService()?;
            let mut frequency = 0_i64;
            QueryPerformanceFrequency(&mut frequency);
            anyhow::ensure!(frequency > 0, "Недоступен аудиотаймер QPC");
            client.Start()?;
            let _ = tx.send(Ok(()));
            let handles = [stop.0, changed.0, ready.0];
            let mut clock = PacketClock::default();
            let result = (|| -> Result<bool> {
                loop {
                    match WaitForMultipleObjects(
                        handles.len() as u32,
                        handles.as_ptr(),
                        0,
                        INFINITE,
                    ) {
                        WAIT_OBJECT_0 => return Ok(false),
                        n if n == WAIT_OBJECT_0 + 1 => return Ok(true),
                        n if n == WAIT_OBJECT_0 + 2 => {}
                        _ => anyhow::bail!("Ошибка ожидания аудиокадра"),
                    }
                    while capture.GetNextPacketSize()? > 0 {
                        let (mut ptr, mut frames, mut flags, mut qpc, mut device) =
                            (std::ptr::null_mut(), 0, 0, 0_u64, 0_u64);
                        capture.GetBuffer(
                            &mut ptr,
                            &mut frames,
                            &mut flags,
                            Some(&mut device),
                            Some(&mut qpc),
                        )?;
                        let length = frames as usize * FRAME_BYTES;
                        let data =
                            if flags & AUDCLNT_BUFFERFLAGS_SILENT.0 as u32 != 0 || ptr.is_null() {
                                vec![0; length]
                            } else {
                                std::slice::from_raw_parts(ptr, length).to_vec()
                            };
                        let mut counter = 0_i64;
                        QueryPerformanceCounter(&mut counter);
                        let age = if flags & AUDCLNT_BUFFERFLAGS_TIMESTAMP_ERROR.0 as u32 == 0 {
                            (counter as f64 / frequency as f64 - qpc as f64 / 10_000_000.0)
                                .clamp(0.0, 2.0)
                        } else {
                            frames as f64 / SAMPLE_RATE as f64
                        };
                        let stamp = Instant::now() - Duration::from_secs_f64(age);
                        let start = clock.packet(
                            stamp,
                            device,
                            frames,
                            flags & AUDCLNT_BUFFERFLAGS_DATA_DISCONTINUITY.0 as u32 != 0,
                        );
                        capture.ReleaseBuffer(frames)?;
                        ring.lock().unwrap().push(start, data);
                    }
                }
            })();
            let _ = client.Stop();
            result
        }
    }

    pub(super) fn run(
        ring: Arc<Mutex<Ring>>,
        stop: Arc<Event>,
        tx: mpsc::Sender<std::result::Result<(), String>>,
        pid: Option<u32>,
    ) {
        let result = (|| -> Result<()> {
            unsafe {
                // Give the event-driven capture worker the standard Windows
                // audio scheduling class without raising the entire app priority.
                use windows_sys::Win32::System::Threading::{
                    AvRevertMmThreadCharacteristics, AvSetMmThreadCharacteristicsW,
                };
                struct AudioPriority(HANDLE);
                impl Drop for AudioPriority {
                    fn drop(&mut self) {
                        if !self.0.is_null() {
                            unsafe {
                                AvRevertMmThreadCharacteristics(self.0);
                            }
                        }
                    }
                }
                let mut task = 0;
                let _priority = AudioPriority(AvSetMmThreadCharacteristicsW(
                    windows_sys::w!("Audio"),
                    &mut task,
                ));
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
                let changed = Arc::new(Event::new()?);
                let notification: IMMNotificationClient = Notifications {
                    changed: changed.clone(),
                }
                .into();
                enumerator.RegisterEndpointNotificationCallback(&notification)?;
                loop {
                    match capture(&enumerator, &ring, &stop, &changed, &tx, pid) {
                        Ok(false) => break,
                        Ok(true) => continue,
                        Err(e) => {
                            let _ = tx.send(Err(format!("{e:#}")));
                            if WaitForSingleObject(stop.0, 2000) == WAIT_OBJECT_0 {
                                break;
                            }
                        }
                    }
                }
                let _ = enumerator.UnregisterEndpointNotificationCallback(&notification);
                Ok(())
            }
        })();
        if let Err(e) = result {
            let _ = tx.send(Err(format!("{e:#}")));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn packet_clock_ignores_callback_jitter() {
        let start = Instant::now();
        let mut clock = PacketClock::default();
        for i in 0..4500_u64 {
            let jitter = Duration::from_micros((i * 137) % 2500);
            let stamp = start + Duration::from_millis(i * 10) + jitter;
            assert_eq!(
                clock.packet(stamp, i * 480, 480, false),
                start + Duration::from_millis(i * 10)
            );
        }
        assert_eq!(
            clock.packet(start + Duration::from_secs(48), 0, 480, true),
            start + Duration::from_secs(48)
        );
    }
    #[cfg(windows)]
    #[test]
    #[ignore = "plays a quiet tone to test native process-loopback"]
    fn windows_process_loopback_isolated_tone() {
        let mut child = crate::media::command(crate::media::tool("ffplay"))
            .args([
                "-nodisp",
                "-autoexit",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=600:duration=4,volume=0.15",
            ])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let recorder = AudioRecorder::for_process(child.id()).unwrap();
        let status = recorder
            .events
            .recv_timeout(Duration::from_secs(6))
            .unwrap();
        if let Err(error) = status {
            crate::media::stop_child(&mut child);
            panic!("{error}");
        }
        std::thread::sleep(Duration::from_secs(2));
        let snapshot = recorder.snapshot(Instant::now(), 1.0);
        crate::media::stop_child(&mut child);
        assert!(snapshot.is_audible(), "No process audio captured");
        let mut data = Vec::new();
        snapshot.write_to(&mut data).unwrap();
        assert_eq!(data.len(), SAMPLE_RATE * FRAME_BYTES);
        assert!(
            data.chunks_exact(4)
                .any(|v| f32::from_le_bytes(v.try_into().unwrap()).abs() > 0.0001)
        );
    }
    #[test]
    fn snapshot_trims_packets_and_fills_device_gaps_with_silence() {
        let start = Instant::now();
        let chunks = vec![
            Chunk {
                start: start - Duration::from_secs(1),
                data: vec![1; 2 * SAMPLE_RATE * FRAME_BYTES].into(),
            },
            Chunk {
                start: start + Duration::from_secs(2),
                data: vec![2; SAMPLE_RATE * FRAME_BYTES].into(),
            },
        ];
        let snapshot = AudioSnapshot {
            chunks,
            start,
            frames: 4 * SAMPLE_RATE,
        };
        let mut bytes = Vec::new();
        snapshot.write_to(&mut bytes).unwrap();
        assert_eq!(bytes.len(), 4 * SAMPLE_RATE * FRAME_BYTES);
        for (second, value) in [(0, 1), (1, 0), (2, 2), (3, 0)] {
            assert!(
                bytes[second * SAMPLE_RATE * FRAME_BYTES..(second + 1) * SAMPLE_RATE * FRAME_BYTES]
                    .iter()
                    .all(|byte| *byte == value)
            );
        }
    }

    fn exact_fixture(encoder: crate::settings::Encoder) {
        use crate::{
            media,
            recorder::{self, SaveOptions},
            replay::{REPLAY_BYTE_LIMIT, ReplayBuffer, TS_PACKET_SIZE},
        };
        use std::sync::atomic::AtomicBool;
        if media::check_tools("ffmpeg", "ffprobe").is_err() {
            return;
        }
        let work = media::WorkDir::new(&std::env::temp_dir(), "rewind-exact-av").unwrap();
        let source = "color=red:size=160x96:rate=60,drawbox=color=blue:t=fill:enable='gte(t,1.5)',drawbox=color=green:t=fill:enable='gte(t,45.5)'";
        let stream = media::command("ffmpeg")
            .args([
                "-v",
                "error",
                "-f",
                "lavfi",
                "-i",
                source,
                "-t",
                "46.5",
                "-c:v",
                "libx264",
                "-preset",
                "ultrafast",
                "-g",
                "120",
                "-bf",
                "0",
                "-f",
                "mpegts",
                "-muxdelay",
                "0",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(
            stream.status.success(),
            "{}",
            String::from_utf8_lossy(&stream.stderr)
        );
        let mut ring = ReplayBuffer::new(45, 60, REPLAY_BYTE_LIMIT);
        for packet in stream.stdout.chunks_exact(TS_PACKET_SIZE) {
            ring.push_packet(packet.try_into().unwrap()).unwrap();
        }
        ring.finish_stream();
        let mut snapshot = ring.snapshot().unwrap();
        assert_eq!(snapshot.duration, 45.0);
        assert!((snapshot.trim_start - 1.5).abs() < 0.001);
        snapshot.source = Some("Deadlock test".into());
        let end = Instant::now();
        snapshot.end_time = end;
        let mut pcm = Vec::with_capacity((46.5 * SAMPLE_RATE as f64) as usize * FRAME_BYTES);
        for i in 0..(46.5 * SAMPLE_RATE as f64) as usize {
            let time = i as f64 / SAMPLE_RATE as f64;
            let frequency = if time < 1.5 {
                100.0
            } else if time < 44.5 {
                440.0
            } else {
                880.0
            };
            let sample = (0.2 * (std::f64::consts::TAU * frequency * time).sin()) as f32;
            pcm.extend_from_slice(&sample.to_le_bytes());
            pcm.extend_from_slice(&sample.to_le_bytes());
        }
        let audio = AudioSnapshot {
            chunks: vec![Chunk {
                start: end - Duration::from_secs_f64(46.5),
                data: pcm.into(),
            }],
            start: end - Duration::from_secs(45),
            frames: 45 * SAMPLE_RATE,
        };
        let mut frozen = snapshot.clone();
        frozen.trim_start = 80.0;
        let frozen_audio = audio.clone();
        let output = work.0.join("exact.mp4");
        recorder::save_snapshot_with_options(
            "ffmpeg",
            snapshot,
            &output,
            Arc::new(AtomicBool::new(false)),
            SaveOptions {
                encoder,
                audio: None,
                tracks: vec![("Deadlock".into(), audio)],
            },
        )
        .unwrap();
        let clip = media::probe("ffprobe", &output).unwrap();
        assert!((clip.duration - 45.0).abs() < 0.002, "{}", clip.duration);
        assert!(clip.has_audio);
        assert_eq!(clip.audio_mix_index, Some(0));
        assert_eq!(clip.audio_tracks.len(), 1);
        assert_eq!(clip.audio_tracks[0].name, "Deadlock");
        assert_eq!(clip.source.as_deref(), Some("Deadlock test"));
        let info = media::command("ffprobe")
            .args([
                "-v",
                "error",
                "-count_frames",
                "-show_streams",
                "-of",
                "json",
            ])
            .arg(&output)
            .output()
            .unwrap();
        let info: serde_json::Value = serde_json::from_slice(&info.stdout).unwrap();
        let video = info["streams"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stream| stream["codec_type"] == "video")
            .unwrap();
        let sound = info["streams"]
            .as_array()
            .unwrap()
            .iter()
            .find(|stream| stream["codec_type"] == "audio")
            .unwrap();
        assert_eq!(video["nb_read_frames"], "2700");
        if encoder == crate::settings::Encoder::Nvidia {
            assert!(
                video["tags"]["encoder"]
                    .as_str()
                    .unwrap_or("")
                    .contains("h264_nvenc"),
                "GPU encoder fallback was used"
            );
        }
        assert_eq!(sound["sample_rate"], "48000");
        assert!((sound["duration"].as_str().unwrap().parse::<f64>().unwrap() - 45.0).abs() < 0.002);
        for (time, blue) in [(0.1, true), (43.8, true), (44.2, false)] {
            let pixel = media::command("ffmpeg")
                .args(["-v", "error", "-ss", &time.to_string(), "-i"])
                .arg(&output)
                .args([
                    "-vf",
                    "scale=1:1",
                    "-frames:v",
                    "1",
                    "-pix_fmt",
                    "rgb24",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ])
                .output()
                .unwrap();
            assert!(pixel.status.success());
            assert!(pixel.stdout.len() >= 3);
            if blue {
                assert!(
                    pixel.stdout[2] > 200 && pixel.stdout[1] < 30,
                    "{time}: {:?}",
                    pixel.stdout
                );
            } else {
                assert!(
                    pixel.stdout[1] > 80 && pixel.stdout[2] < 30,
                    "{time}: {:?}",
                    pixel.stdout
                );
            }
        }
        for (time, frequency) in [(0.1, 440.0), (44.6, 880.0)] {
            let sound = media::command("ffmpeg")
                .args(["-v", "error", "-ss", &time.to_string(), "-i"])
                .arg(&output)
                .args([
                    "-t", "0.2", "-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le",
                    "pipe:1",
                ])
                .output()
                .unwrap();
            assert!(sound.status.success());
            let samples: Vec<_> = sound
                .stdout
                .chunks_exact(4)
                .map(|s| f32::from_le_bytes(s.try_into().unwrap()))
                .collect();
            let crossings = samples
                .windows(2)
                .filter(|pair| pair[0] < 0.0 && pair[1] >= 0.0)
                .count() as f64;
            let measured = crossings / (samples.len() as f64 / SAMPLE_RATE as f64);
            assert!((measured - frequency).abs() < 12.0, "{time}: {measured} Hz");
        }
        let decoded = media::command("ffmpeg")
            .args(["-v", "error", "-xerror", "-i"])
            .arg(&output)
            .args([
                "-f",
                "null",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            ])
            .output()
            .unwrap();
        assert!(
            decoded.status.success(),
            "{}",
            String::from_utf8_lossy(&decoded.stderr)
        );
        // Even a window wholly after the last available video frame must
        // remain 45 seconds long and retain new audio while holding that frame.
        if encoder == crate::settings::Encoder::Cpu {
            let held = work.0.join("held.mp4");
            recorder::save_snapshot_with_options(
                "ffmpeg",
                frozen,
                &held,
                Arc::new(AtomicBool::new(false)),
                SaveOptions {
                    encoder,
                    audio: Some(frozen_audio),
                    tracks: Vec::new(),
                },
            )
            .unwrap();
            let clip = media::probe("ffprobe", &held).unwrap();
            assert!((clip.duration - 45.0).abs() < 0.002);
            assert!(clip.has_audio);
            let pixel = media::command("ffmpeg")
                .args(["-v", "error", "-i"])
                .arg(&held)
                .args([
                    "-vf",
                    "scale=1:1",
                    "-frames:v",
                    "1",
                    "-pix_fmt",
                    "rgb24",
                    "-f",
                    "rawvideo",
                    "pipe:1",
                ])
                .output()
                .unwrap();
            assert!(pixel.status.success());
            assert!(
                pixel.stdout[1] > 80 && pixel.stdout[2] < 30,
                "Last green frame was not held"
            );
        }
    }

    #[test]
    fn exact_45_seconds_keeps_nonkeyframe_start_tail_and_synced_audio() {
        exact_fixture(crate::settings::Encoder::Cpu);
    }

    #[test]
    #[ignore = "Requires NVIDIA NVDEC/NVENC; run explicitly"]
    fn exact_45_seconds_on_gpu_with_audio() {
        exact_fixture(crate::settings::Encoder::Nvidia);
    }

    #[test]
    #[ignore = "Plays a quiet two-second test tone and captures Windows output"]
    fn windows_loopback_captures_real_system_sound() {
        let recorder = AudioRecorder::new().unwrap();
        recorder
            .events
            .recv_timeout(Duration::from_secs(5))
            .unwrap()
            .unwrap();
        let status = crate::media::command("ffplay")
            .args([
                "-v",
                "error",
                "-nodisp",
                "-autoexit",
                "-f",
                "lavfi",
                "-i",
                "sine=frequency=600:duration=2:sample_rate=48000",
                "-af",
                "volume=0.03",
            ])
            .status()
            .unwrap();
        assert!(status.success());
        std::thread::sleep(Duration::from_millis(100));
        let snapshot = recorder.snapshot(Instant::now(), 3.0);
        let mut pcm = Vec::new();
        snapshot.write_to(&mut pcm).unwrap();
        let count = pcm
            .chunks_exact(4)
            .filter(|sample| f32::from_le_bytes((*sample).try_into().unwrap()).abs() > 0.00001)
            .count();
        assert!(
            count > 20_000,
            "No real sound captured: {count} nonzero samples"
        );
        assert_eq!(pcm.len(), 3 * SAMPLE_RATE * FRAME_BYTES);
    }
}
