use crate::{
    audio::AudioSnapshot,
    audio_apps::ApplicationAudio,
    media::{self, WorkDir},
    replay::{REPLAY_BYTE_LIMIT, ReplayBuffer, ReplaySnapshot, TS_PACKET_SIZE},
    settings::{Encoder, Settings},
};
use anyhow::{Context, Result, bail};
use std::{
    fs,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
    time::{Duration, Instant},
};

pub enum Request {
    Start(Settings),
    Stop,
    Configure {
        seconds: u32,
        output_dir: PathBuf,
        system_audio: bool,
    },
    Save,
    Shutdown,
}
pub enum Event {
    Started(String),
    Stopped,
    Buffer(f64),
    Saving,
    Saved(Result<PathBuf, String>),
    CaptureFailed { message: String, temporary: bool },
    Error(String),
    AudioState(std::result::Result<(), String>),
}

/// Desktop transitions invalidate DXGI duplication; recreate it instead of
/// treating this as an encoder failure and falling back to CPU capture.
pub fn is_desktop_transition(message: &str) -> bool {
    let message = message.to_ascii_lowercase();
    [
        "887a0026",
        "dxgi_error_access_lost",
        "887a0028",
        "dxgi_error_session_disconnected",
    ]
    .iter()
    .any(|code| message.contains(code))
}

fn capture_failure(message: String) -> Event {
    Event::CaptureFailed {
        temporary: is_desktop_transition(&message),
        message,
    }
}

pub struct Recorder {
    pub tx: Sender<Request>,
    pub rx: Receiver<Event>,
    shutdown: Arc<AtomicBool>,
    worker: Option<thread::JoinHandle<()>>,
}
impl Recorder {
    pub fn new(wake: Arc<dyn Fn() + Send + Sync>) -> Self {
        let (tx, requests) = mpsc::channel();
        let (events, rx) = mpsc::channel();
        let shutdown = Arc::new(AtomicBool::new(false));
        let flag = shutdown.clone();
        let worker = thread::spawn(move || worker(requests, events, flag, wake));
        Self {
            tx,
            rx,
            shutdown,
            worker: Some(worker),
        }
    }
}
impl Drop for Recorder {
    fn drop(&mut self) {
        self.shutdown.store(true, Ordering::Relaxed);
        let _ = self.tx.send(Request::Shutdown);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

struct Session {
    child: Child,
    buffer: Arc<Mutex<ReplayBuffer>>,
    log: Arc<Mutex<Vec<u8>>>,
    stream_error: Arc<Mutex<Option<String>>>,
    reader: Option<thread::JoinHandle<()>>,
    logger: Option<thread::JoinHandle<()>>,
    settings: Settings,
    encoder: String,
    encoder_kind: Encoder,
}

impl Session {
    fn attach(
        mut child: Child,
        settings: Settings,
        encoder: Encoder,
        buffer: Arc<Mutex<ReplayBuffer>>,
    ) -> Self {
        let mut stdout = child.stdout.take().unwrap();
        let mut stderr = child.stderr.take().unwrap();
        buffer.lock().unwrap().begin_stream();
        let log = Arc::new(Mutex::new(Vec::new()));
        let stream_error = Arc::new(Mutex::new(None));
        let target = buffer.clone();
        let errors = stream_error.clone();
        let reader = thread::spawn(move || {
            // Pipe reads can end between TS packets. Keep at most 187 carry bytes
            // and lock once per read, rather than once per transport packet.
            let mut input = [0_u8; TS_PACKET_SIZE * 256];
            let mut pending = Vec::with_capacity(input.len() + TS_PACKET_SIZE);
            let result = (|| -> Result<()> {
                loop {
                    let count = match stdout.read(&mut input) {
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                        result => result?,
                    };
                    if count == 0 {
                        break;
                    }
                    pending.extend_from_slice(&input[..count]);
                    let consumed = pending.len() / TS_PACKET_SIZE * TS_PACKET_SIZE;
                    {
                        let mut ring = target.lock().unwrap();
                        for packet in pending[..consumed].chunks_exact(TS_PACKET_SIZE) {
                            ring.push_packet(packet.try_into().unwrap())?;
                        }
                    }
                    pending.copy_within(consumed.., 0);
                    pending.truncate(pending.len() - consumed);
                }
                // An interrupted capture can end in a partial access unit.
                // Never commit that final pending frame to a live snapshot.
                Ok(())
            })();
            if let Err(error) = result {
                *errors.lock().unwrap() = Some(format!("{error:#}"));
            }
        });
        let messages = log.clone();
        let logger = thread::spawn(move || {
            let mut bytes = [0_u8; 2048];
            while let Ok(count) = stderr.read(&mut bytes) {
                if count == 0 {
                    break;
                }
                let mut log = messages.lock().unwrap();
                log.extend_from_slice(&bytes[..count]);
                if log.len() > 16 * 1024 {
                    let excess = log.len() - 16 * 1024;
                    log.drain(..excess);
                }
            }
        });
        Self {
            child,
            buffer,
            log,
            stream_error,
            reader: Some(reader),
            logger: Some(logger),
            settings,
            encoder: encoder.label().into(),
            encoder_kind: encoder,
        }
    }

    fn error_message(&self) -> String {
        let message = self.stream_error.lock().unwrap().clone();
        message.unwrap_or_else(|| String::from_utf8_lossy(&self.log.lock().unwrap()).into_owned())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        media::stop_child(&mut self.child);
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
        if let Some(logger) = self.logger.take() {
            let _ = logger.join();
        }
    }
}

fn graphics_capture_available(ffmpeg: &str) -> bool {
    media::command(media::tool(ffmpeg))
        .args(["-hide_banner", "-h", "filter=gfxcapture"])
        .stdin(Stdio::null())
        .output()
        .is_ok_and(|output| {
            output.status.success()
                && String::from_utf8_lossy(&output.stdout).contains("Filter gfxcapture")
        })
}

fn graphics_source(settings: &Settings) -> Result<String> {
    #[cfg(windows)]
    unsafe {
        use windows::Win32::Graphics::Dxgi::{CreateDXGIFactory1, IDXGIFactory1};
        let factory: IDXGIFactory1 = CreateDXGIFactory1()?;
        let desc = factory
            .EnumAdapters1(0)?
            .EnumOutputs(settings.monitor)?
            .GetDesc()?;
        Ok(format!(
            "gfxcapture=hmonitor={}:max_framerate={}:capture_cursor=1:display_border=0:width={}:height={}:resize_mode=scale_aspect",
            desc.Monitor.0 as usize,
            settings.fps,
            desc.DesktopCoordinates.right - desc.DesktopCoordinates.left,
            desc.DesktopCoordinates.bottom - desc.DesktopCoordinates.top
        ))
    }
    #[cfg(not(windows))]
    bail!("Windows Graphics Capture доступен только в Windows")
}

fn spawn_capture(
    settings: &Settings,
    encoder: Encoder,
    legacy: bool,
    graphics: Option<&str>,
) -> Result<Child> {
    let mut cmd = media::command(media::tool(&settings.ffmpeg));
    cmd.args(["-hide_banner", "-loglevel", "warning", "-nostdin"]);
    cmd.args(["-copyts", "-use_wallclock_as_timestamps", "1"]);
    if legacy {
        if settings.monitor != 0 {
            bail!("GDI поддерживает только основной экран");
        }
        cmd.args([
            "-f",
            "gdigrab",
            "-framerate",
            &settings.fps.to_string(),
            "-i",
            "desktop",
        ]);
    } else if let Some(source) = graphics {
        cmd.args(["-f", "lavfi", "-i", source]);
    } else {
        cmd.args([
            "-f",
            "lavfi",
            "-i",
            &format!(
                "ddagrab=output_idx={}:framerate={}:draw_mouse=1",
                settings.monitor, settings.fps
            ),
        ]);
    }
    match encoder {
        Encoder::Nvidia => {
            cmd.args([
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p1",
                "-tune",
                "ll",
                "-forced-idr",
                "1",
                "-b:v",
                "8M",
                "-maxrate",
                "12M",
                "-bufsize",
                "16M",
            ]);
        }
        other => {
            let filter = if legacy {
                "scale=trunc(iw/2)*2:trunc(ih/2)*2,format=nv12"
            } else {
                "hwdownload,format=bgra,scale=trunc(iw/2)*2:trunc(ih/2)*2,format=nv12"
            };
            cmd.args(["-vf", filter]);
            match other {
                Encoder::Intel => {
                    cmd.args(["-c:v", "h264_qsv", "-preset", "veryfast", "-b:v", "8M"]);
                }
                Encoder::Amd => {
                    cmd.args(["-c:v", "h264_amf", "-quality", "speed", "-b:v", "8M"]);
                }
                _ => {
                    cmd.args([
                        "-c:v",
                        "libx264",
                        "-preset",
                        "ultrafast",
                        "-tune",
                        "zerolatency",
                        "-crf",
                        "23",
                        "-threads",
                        "2",
                    ]);
                }
            }
        }
    }
    // A continuous hardware-encoded stream. GOPs are for independently decodable
    // replay boundaries, not file segmentation; there is no periodic file I/O.
    cmd.args([
        "-an",
        "-g",
        &(settings.fps * 2).to_string(),
        "-keyint_min",
        &(settings.fps * 2).to_string(),
        "-bf",
        "0",
        "-force_key_frames",
        "expr:if(isnan(prev_forced_t),1,gte(t,prev_forced_t+2))",
        "-f",
        "mpegts",
        "-mpegts_start_pid",
        "256",
        "-mpegts_pmt_start_pid",
        "4096",
        "-pat_period",
        "0.5",
        "-muxdelay",
        "0",
        "-muxpreload",
        "0",
        "-flush_packets",
        "1",
        "pipe:1",
    ])
    .stdin(Stdio::null())
    .stdout(Stdio::piped())
    .stderr(Stdio::piped());
    cmd.spawn().context("Не удалось запустить запись")
}

fn start(
    settings: Settings,
    shutdown: &AtomicBool,
    buffer: Arc<Mutex<ReplayBuffer>>,
    graphics: bool,
) -> Result<Session> {
    let graphics = graphics.then(|| graphics_source(&settings)).transpose()?;
    let candidates = match settings.encoder {
        Encoder::Auto => vec![
            (Encoder::Nvidia, false),
            (Encoder::Intel, false),
            (Encoder::Amd, false),
            (Encoder::Cpu, false),
            (Encoder::Cpu, true),
        ],
        Encoder::Cpu => vec![(Encoder::Cpu, false), (Encoder::Cpu, true)],
        encoder => vec![(encoder, false)],
    };
    let mut errors = Vec::new();
    let candidates: Vec<_> = candidates
        .into_iter()
        .flat_map(|(encoder, legacy)| {
            if graphics.is_some() && !legacy {
                vec![(encoder, false, true), (encoder, false, false)]
            } else {
                vec![(encoder, legacy, false)]
            }
        })
        .collect();
    for (encoder, legacy, use_graphics) in candidates {
        if shutdown.load(Ordering::Relaxed) {
            bail!("Запуск отменён");
        }
        let child = spawn_capture(
            &settings,
            encoder,
            legacy,
            if use_graphics {
                graphics.as_deref()
            } else {
                None
            },
        )?;
        let mut session = Session::attach(child, settings.clone(), encoder, buffer.clone());
        if use_graphics {
            session.encoder.push_str(" · Windows Graphics Capture");
        }
        let started = Instant::now();
        loop {
            if shutdown.load(Ordering::Relaxed) {
                bail!("Запуск отменён");
            }
            if session.child.try_wait()?.is_some() || session.stream_error.lock().unwrap().is_some()
            {
                break;
            }
            if session.buffer.lock().unwrap().stream_duration() >= 0.1 {
                return Ok(session);
            }
            if started.elapsed() > Duration::from_secs(2) {
                // A live source can wait for a screen update (or a sleeping
                // display). Lack of frames is not an encoder failure: keep the
                // GPU process and replay history instead of falling back to GDI.
                return Ok(session);
            }
            thread::sleep(Duration::from_millis(40));
        }
        media::stop_child(&mut session.child);
        if let Some(logger) = session.logger.take() {
            let _ = logger.join();
        }
        let message = session.error_message();
        if is_desktop_transition(&message) {
            bail!("{message}");
        }
        errors.push(format!("{}: {}", encoder.label(), message));
    }
    bail!("Не удалось начать запись.\n{}", errors.join("\n"))
}

#[derive(Default)]
pub struct SaveOptions {
    pub encoder: Encoder,
    pub audio: Option<AudioSnapshot>,
    pub tracks: Vec<(String, AudioSnapshot)>,
}

/// Render exactly the requested window, including a partial leading GOP and
/// held frames during capture transitions. Disk work starts only after Save.
pub fn save_snapshot(
    ffmpeg: &str,
    snapshot: ReplaySnapshot,
    output: &Path,
    cancel: Arc<AtomicBool>,
) -> Result<PathBuf> {
    save_snapshot_with_options(
        ffmpeg,
        snapshot,
        output,
        cancel,
        SaveOptions {
            encoder: Encoder::Cpu,
            audio: None,
            tracks: Vec::new(),
        },
    )
}

pub fn save_snapshot_with_options(
    ffmpeg: &str,
    snapshot: ReplaySnapshot,
    output: &Path,
    cancel: Arc<AtomicBool>,
    options: SaveOptions,
) -> Result<PathBuf> {
    if cancel.load(Ordering::Relaxed) {
        bail!("Операция отменена");
    }
    if output.exists() {
        bail!("Файл уже существует");
    }
    let work = WorkDir::new(output.parent().context("Нет папки клипов")?, ".replay")?;
    let partial = work.0.join("replay.mp4");
    let duration = (snapshot.duration * snapshot.fps as f64).round().max(1.0) / snapshot.fps as f64;
    let separate = !options.tracks.is_empty();
    let tracks = if separate {
        options.tracks
    } else {
        options
            .audio
            .map(|a| ("Системный звук".to_owned(), a))
            .into_iter()
            .collect()
    };
    let mut audio_paths = Vec::new();
    for (index, (name, audio)) in tracks.into_iter().enumerate() {
        let path = work.0.join(format!("audio-{index}.f32"));
        let mut writer = std::io::BufWriter::new(fs::File::create(&path)?);
        audio.write_to(&mut writer)?;
        std::io::Write::flush(&mut writer)?;
        audio_paths.push((name, path));
    }
    let candidates = match options.encoder {
        Encoder::Nvidia => vec![
            (Encoder::Nvidia, true),
            (Encoder::Nvidia, false),
            (Encoder::Cpu, false),
        ],
        Encoder::Intel => vec![(Encoder::Intel, false), (Encoder::Cpu, false)],
        Encoder::Amd => vec![(Encoder::Amd, false), (Encoder::Cpu, false)],
        _ => vec![(Encoder::Cpu, false)],
    };
    let mut failure = None;
    for (encoder, decode_gpu) in candidates {
        match render_snapshot(
            ffmpeg,
            &snapshot,
            &partial,
            &work.0,
            &cancel,
            encoder,
            decode_gpu,
            &audio_paths,
            separate,
            duration,
        ) {
            Ok(()) => {
                if cancel.load(Ordering::Relaxed) {
                    bail!("Операция отменена");
                }
                media::publish(&partial, output)?;
                return Ok(output.to_path_buf());
            }
            Err(e) => {
                failure = Some(e);
                if cancel.load(Ordering::Relaxed) {
                    break;
                }
            }
        }
    }
    Err(failure.unwrap_or_else(|| anyhow::anyhow!("Не удалось сохранить клип")))
}

#[allow(clippy::too_many_arguments)]
fn render_snapshot(
    ffmpeg: &str,
    snapshot: &ReplaySnapshot,
    partial: &Path,
    work: &Path,
    cancel: &AtomicBool,
    encoder: Encoder,
    decode_gpu: bool,
    audio: &[(String, PathBuf)],
    separate: bool,
    duration: f64,
) -> Result<()> {
    let log_path = work.join("ffmpeg.log");
    let mut command = media::command(media::tool(ffmpeg));
    command.args([
        "-hide_banner",
        "-loglevel",
        "error",
        "-y",
        "-nostdin",
        "-filter_threads",
        "1",
        "-copyts",
        "-start_at_zero",
    ]);
    if decode_gpu {
        command.args(["-hwaccel", "cuda", "-hwaccel_output_format", "cuda"]);
    } else {
        command.args(["-threads", "2"]);
    }
    command.args(["-f", "mpegts", "-i", "pipe:0"]);
    for (_, path) in audio {
        command
            .args(["-f", "f32le", "-ar", "48000", "-ac", "2", "-i"])
            .arg(path);
    }
    command.args(["-map", "0:v:0"]);
    if separate {
        let inputs = (1..=audio.len())
            .map(|i| format!("[{i}:a:0]"))
            .collect::<String>();
        let filter = format!(
            "{inputs}amix=inputs={}:normalize=0:dropout_transition=0,alimiter=limit=0.95:level=0:latency=1[mix]",
            audio.len()
        );
        command.args([
            "-filter_complex_threads",
            "1",
            "-filter_complex",
            &filter,
            "-map",
            "[mix]",
            "-metadata:s:a:0",
            "handler_name=Rewind mix",
            "-metadata:s:a:0",
            "title=Все приложения",
            "-disposition:a:0",
            "default",
        ]);
        for (i, (name, _)) in audio.iter().enumerate() {
            command.args([
                "-map",
                &format!("{}:a:0", i + 1),
                &format!("-metadata:s:a:{}", i + 1),
                &format!("handler_name={name}"),
                &format!("-metadata:s:a:{}", i + 1),
                &format!("title={name}"),
                &format!("-disposition:a:{}", i + 1),
                "0",
            ]);
        }
        command.args(["-c:a", "aac", "-b:a", "160k"]);
    } else if !audio.is_empty() {
        command.args(["-map", "1:a:0", "-c:a", "aac", "-b:a", "160k"]);
    } else {
        command.arg("-an");
    }
    let padding = snapshot.trim_start + duration + 1.0;
    let filter = format!(
        "tpad=stop_mode=clone:stop_duration={padding:.9},fps=fps={}:start_time={:.9}:round=near,setpts=PTS-STARTPTS",
        snapshot.fps, snapshot.trim_start
    );
    command.args([
        "-vf",
        &filter,
        "-frames:v",
        &((duration * snapshot.fps as f64).round() as u64).to_string(),
        "-t",
        &format!("{duration:.9}"),
        "-bf",
        "0",
    ]);
    match encoder {
        Encoder::Nvidia => {
            command.args([
                "-c:v",
                "h264_nvenc",
                "-preset",
                "p1",
                "-rc",
                "constqp",
                "-qp",
                "18",
            ]);
        }
        Encoder::Intel => {
            command.args(["-c:v", "h264_qsv", "-preset", "veryfast", "-b:v", "8M"]);
        }
        Encoder::Amd => {
            command.args(["-c:v", "h264_amf", "-quality", "speed", "-b:v", "8M"]);
        }
        _ => {
            command.args([
                "-c:v", "libx264", "-preset", "veryfast", "-crf", "18", "-threads", "2",
            ]);
        }
    }
    command.args([
        "-pix_fmt",
        if decode_gpu {
            "cuda"
        } else if matches!(encoder, Encoder::Intel | Encoder::Amd) {
            "nv12"
        } else {
            "yuv420p"
        },
        "-movflags",
        "+faststart",
    ]);
    if let Some(source) = &snapshot.source {
        command.args(["-metadata", &format!("comment=Rewind source: {source}")]);
    }
    let mut child = command
        .arg(partial)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(fs::File::create(&log_path)?)
        .spawn()
        .context("Не удалось запустить сохранение клипа")?;
    let mut input = child.stdin.take().unwrap();
    let data = snapshot.clone();
    let writer = thread::spawn(move || data.write_to(&mut input));
    let result = loop {
        if cancel.load(Ordering::Relaxed) {
            media::stop_child(&mut child);
            break Err(anyhow::anyhow!("Операция отменена"));
        }
        match child.try_wait() {
            Ok(Some(status)) if status.success() => break Ok(()),
            Ok(Some(_)) => {
                break Err(anyhow::anyhow!(
                    "Сохранение клипа: {}",
                    fs::read_to_string(&log_path).unwrap_or_default()
                ));
            }
            Err(error) => {
                media::stop_child(&mut child);
                break Err(error.into());
            }
            Ok(None) => thread::sleep(Duration::from_millis(40)),
        }
    };
    let written = writer
        .join()
        .map_err(|_| anyhow::anyhow!("Прерван поток сохранения"))?;
    result?;
    // A duration/frame limit can legitimately close stdin before the last
    // transport padding packets. A successful renderer is authoritative here.
    if let Err(e) = written
        && e.kind() != std::io::ErrorKind::BrokenPipe
    {
        return Err(e).context("Не удалось передать буфер в FFmpeg");
    }
    if cancel.load(Ordering::Relaxed) {
        bail!("Операция отменена");
    }
    Ok(())
}

fn worker(
    requests: Receiver<Request>,
    events: Sender<Event>,
    shutdown: Arc<AtomicBool>,
    wake: Arc<dyn Fn() + Send + Sync>,
) {
    let mut session: Option<Session> = None;
    let mut replay: Option<Arc<Mutex<ReplayBuffer>>> = None;
    let mut recording_settings: Option<Settings> = None;
    let mut applications: Option<ApplicationAudio> = None;
    let mut recording_encoder = Encoder::Cpu;
    let mut graphics_support: Option<(String, bool)> = None;
    let saving = Arc::new(AtomicBool::new(false));
    let emit = |event| {
        let _ = events.send(event);
        wake();
    };
    let mut last_buffer = -1.0_f64;
    let mut save_worker: Option<thread::JoinHandle<()>> = None;
    while !shutdown.load(Ordering::Relaxed) {
        match requests.recv_timeout(Duration::from_millis(200)) {
            Ok(Request::Shutdown) | Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Ok(Request::Start(settings)) => {
                if session.is_none() {
                    let buffer = replay
                        .get_or_insert_with(|| {
                            let mut buffer = ReplayBuffer::new(
                                settings.buffer_seconds,
                                settings.fps,
                                REPLAY_BYTE_LIMIT,
                            );
                            buffer.enable_wallclock_timestamps();
                            Arc::new(Mutex::new(buffer))
                        })
                        .clone();
                    if settings.system_audio {
                        applications
                            .get_or_insert_with(|| ApplicationAudio::new(settings.buffer_seconds));
                    }
                    if graphics_support
                        .as_ref()
                        .is_none_or(|(path, _)| *path != settings.ffmpeg)
                    {
                        graphics_support = Some((
                            settings.ffmpeg.clone(),
                            graphics_capture_available(&settings.ffmpeg),
                        ));
                    }
                    recording_settings = Some(settings.clone());
                    match start(
                        settings,
                        &shutdown,
                        buffer,
                        graphics_support.as_ref().unwrap().1,
                    ) {
                        Ok(active) => {
                            recording_encoder = active.encoder_kind;
                            last_buffer = -1.0;
                            emit(Event::Started(active.encoder.clone()));
                            session = Some(active);
                        }
                        Err(error) => emit(capture_failure(format!("{error:#}"))),
                    }
                }
            }
            Ok(Request::Stop) => {
                session = None;
                replay = None;
                recording_settings = None;
                applications = None;
                last_buffer = -1.0;
                emit(Event::Stopped);
            }
            Ok(Request::Configure {
                seconds,
                output_dir,
                system_audio,
            }) => {
                if !system_audio {
                    applications = None;
                } else if replay.is_some() {
                    applications.get_or_insert_with(|| ApplicationAudio::new(seconds));
                }
                if let Some(capture) = &applications {
                    capture.set_seconds(seconds);
                }
                if let Some(buffer) = &replay {
                    buffer.lock().unwrap().set_seconds(seconds);
                }
                if let Some(settings) = &mut recording_settings {
                    settings.buffer_seconds = seconds;
                    settings.output_dir = output_dir.clone();
                    settings.system_audio = system_audio;
                }
                if let Some(active) = &mut session {
                    active.settings.buffer_seconds = seconds;
                    active.settings.output_dir = output_dir;
                }
            }
            Ok(Request::Save) => {
                if let (Some(buffer), Some(settings)) = (&replay, &recording_settings) {
                    if saving.load(Ordering::Relaxed) {
                        continue;
                    }
                    let snapshot = buffer.lock().unwrap().live_snapshot(Instant::now());
                    match snapshot {
                        Err(error) => emit(Event::Error(error.to_string())),
                        Ok(mut snapshot) => {
                            snapshot.source = media::foreground_app_name();
                            let duration =
                                (snapshot.duration * snapshot.fps as f64).round().max(1.0)
                                    / snapshot.fps as f64;
                            let options = SaveOptions {
                                encoder: recording_encoder,
                                tracks: applications
                                    .as_ref()
                                    .map(|a| a.snapshots(snapshot.end_time, duration))
                                    .unwrap_or_default(),
                                // A system-wide mix would bypass the Steam exclusion.
                                audio: None,
                            };
                            if let Some(worker) = save_worker.take() {
                                let _ = worker.join();
                            }
                            saving.store(true, Ordering::Relaxed);
                            emit(Event::Saving);
                            let flag = saving.clone();
                            let tx = events.clone();
                            let wake = wake.clone();
                            let ffmpeg = settings.ffmpeg.clone();
                            let cancel = shutdown.clone();
                            let output = media::unique_path(&settings.output_dir, "Replay", "mp4");
                            save_worker = Some(thread::spawn(move || {
                                let result = save_snapshot_with_options(
                                    &ffmpeg, snapshot, &output, cancel, options,
                                )
                                .map_err(|e| format!("{e:#}"));
                                flag.store(false, Ordering::Relaxed);
                                let _ = tx.send(Event::Saved(result));
                                wake();
                            }));
                        }
                    }
                }
            }
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if let Some(capture) = &applications {
            while let Ok(state) = capture.events.try_recv() {
                emit(Event::AudioState(state));
            }
        }
        if let Some(active) = &mut session
            && (active.child.try_wait().is_ok_and(|status| status.is_some())
                || active.stream_error.lock().unwrap().is_some())
        {
            media::stop_child(&mut active.child);
            if let Some(logger) = active.logger.take() {
                let _ = logger.join();
            }
            let message = active.error_message();
            session = None;
            last_buffer = -1.0;
            emit(capture_failure(format!("Запись остановилась: {message}")));
            continue;
        }
        if let (Some(buffer), Some(settings)) = (&replay, &recording_settings) {
            let buffered = buffer
                .lock()
                .unwrap()
                .live_duration(Instant::now())
                .min(settings.buffer_seconds as f64)
                .floor();
            if buffered != last_buffer {
                last_buffer = buffered;
                emit(Event::Buffer(buffered));
            }
        }
    }
    drop(session);
    shutdown.store(true, Ordering::Relaxed);
    if let Some(worker) = save_worker {
        let _ = worker.join();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn desktop_loss_is_recoverable_but_encoder_and_stream_failures_are_not() {
        for message in [
            "[Parsed_ddagrab_0] AcquireNextFrame failed: 887A0026\nEOF timestamp not reliable",
            "AcquireNextFrame failed: 0x887a0026",
            "DXGI_ERROR_ACCESS_LOST",
            "DXGI_ERROR_SESSION_DISCONNECTED",
        ] {
            assert!(matches!(
                capture_failure(message.into()),
                Event::CaptureFailed {
                    temporary: true,
                    ..
                }
            ));
        }
        for message in [
            "No NVENC capable devices found",
            "Unknown encoder",
            "Invalid TS packet",
            "",
            "Generic error in an external library",
        ] {
            assert!(matches!(
                capture_failure(message.into()),
                Event::CaptureFailed {
                    temporary: false,
                    ..
                }
            ));
        }
    }
}
