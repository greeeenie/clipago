use anyhow::{Context, Result, bail};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

pub fn command(program: impl AsRef<OsStr>) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // CREATE_NO_WINDOW
    }
    cmd
}

pub fn tool(configured: &str) -> PathBuf {
    if Path::new(configured).components().count() == 1
        && let Ok(exe) = std::env::current_exe()
    {
        for dir in [
            exe.parent().unwrap().to_path_buf(),
            exe.parent().unwrap().join("bin"),
            exe.parent().unwrap().join("bin/ffmpeg"),
        ] {
            let path = dir.join(format!("{}.exe", configured.trim_end_matches(".exe")));
            if path.exists() {
                return path;
            }
        }
    }
    PathBuf::from(configured)
}

pub fn check_tools(ffmpeg: &str, ffprobe: &str) -> Result<()> {
    for name in [ffmpeg, ffprobe] {
        let output = command(tool(name))
            .args(["-version"])
            .output()
            .with_context(|| format!("Не найден {name}. Укажите путь в настройках."))?;
        if !output.status.success() {
            bail!("Не удалось запустить {name}");
        }
    }
    Ok(())
}

/// Probe actual encoder initialization once, off the UI thread, without files.
pub fn available_recording_encoders(ffmpeg: &str) -> Vec<crate::settings::Encoder> {
    use crate::settings::Encoder;
    let mut encoders = vec![Encoder::Auto];
    for (value, name) in [
        (Encoder::Nvidia, "h264_nvenc"),
        (Encoder::Intel, "h264_qsv"),
        (Encoder::Amd, "h264_amf"),
    ] {
        let Ok(mut child) = command(tool(ffmpeg))
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "color=size=160x96:rate=30,format=nv12",
                "-frames:v",
                "1",
                "-c:v",
                name,
                "-f",
                "null",
                "-",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
        else {
            continue;
        };
        let deadline = std::time::Instant::now() + Duration::from_secs(4);
        loop {
            match child.try_wait() {
                Ok(Some(status)) => {
                    if status.success() {
                        encoders.push(value);
                    }
                    break;
                }
                Err(_) => {
                    stop_child(&mut child);
                    break;
                }
                _ if std::time::Instant::now() >= deadline => {
                    stop_child(&mut child);
                    break;
                }
                _ => thread::sleep(Duration::from_millis(30)),
            }
        }
    }
    encoders.push(Encoder::Cpu);
    encoders
}

#[derive(Clone, Debug, Serialize)]
pub struct Clip {
    pub path: PathBuf,
    pub duration: f64,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
    pub has_audio: bool,
    pub audio_tracks: Vec<AudioTrack>,
    pub audio_mix_index: Option<u32>,
    pub source: Option<String>,
    pub game: bool,
    pub fps: f64,
    pub aspect_ratio: f32,
    pub original_path: Option<PathBuf>,
    pub title: Option<String>,
    pub viewed: bool,
    pub is_movie: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct AudioTrack {
    /// Relative audio-stream index, never a container stream index.
    pub index: u32,
    pub name: String,
    #[serde(skip)]
    pub codec: String,
}

impl Clip {
    pub fn name(&self) -> String {
        self.path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }
}

#[derive(Deserialize)]
struct Probe {
    streams: Vec<Stream>,
    format: Format,
}

#[derive(Deserialize)]
struct Stream {
    codec_type: Option<String>,
    codec_name: Option<String>,
    width: Option<u32>,
    height: Option<u32>,
    avg_frame_rate: Option<String>,
    sample_aspect_ratio: Option<String>,
    #[serde(default)]
    side_data_list: Vec<DisplaySideData>,
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
}

#[derive(Deserialize)]
struct DisplaySideData {
    rotation: Option<f64>,
}

fn display_aspect(width: u32, height: u32, sar: Option<&str>, rotation: f64) -> f32 {
    let pixel_aspect = sar
        .and_then(|sar| {
            let (n, d) = sar.split_once(':')?;
            let value = n.parse::<f32>().ok()? / d.parse::<f32>().ok()?;
            (value.is_finite() && value > 0.0).then_some(value)
        })
        .unwrap_or(1.0);
    let aspect = width.max(1) as f32 * pixel_aspect / height.max(1) as f32;
    if (rotation.rem_euclid(180.0) - 90.0).abs() < 1.0 {
        1.0 / aspect
    } else {
        aspect
    }
}

#[derive(Deserialize)]
struct Format {
    duration: Option<String>,
    #[serde(default)]
    tags: std::collections::HashMap<String, String>,
}

pub fn probe(ffprobe: &str, path: &Path) -> Result<Clip> {
    let output = command(tool(ffprobe))
        .args([
            "-v",
            "error",
            "-show_streams",
            "-show_format",
            "-of",
            "json",
        ])
        .arg(path)
        .output()
        .context("Не удалось запустить ffprobe")?;
    if !output.status.success() {
        bail!(
            "Не удалось открыть видео: {}",
            String::from_utf8_lossy(&output.stderr)
        );
    }
    let data: Probe = serde_json::from_slice(&output.stdout)?;
    let video = data
        .streams
        .iter()
        .find(|s| s.codec_type.as_deref() == Some("video"))
        .context("В файле нет видеодорожки")?;
    let duration = data
        .format
        .duration
        .as_deref()
        .unwrap_or("0")
        .parse::<f64>()?;
    if !duration.is_finite() || duration <= 0.0 {
        bail!("Не удалось определить длительность видео");
    }
    let audio: Vec<_> = data
        .streams
        .iter()
        .filter(|s| s.codec_type.as_deref() == Some("audio"))
        .collect();
    let audio_mix_index = (audio.len() > 1)
        .then(|| {
            audio.iter().position(|s| {
                s.tags
                    .get("handler_name")
                    .is_some_and(|n| n == "Rewind mix")
            })
        })
        .flatten()
        .map(|i| i as u32);
    let audio_tracks = audio
        .iter()
        .enumerate()
        .filter(|(i, _)| Some(*i as u32) != audio_mix_index)
        .map(|(i, s)| AudioTrack {
            index: i as u32,
            codec: s.codec_name.clone().unwrap_or_default(),
            name: s
                .tags
                .get("title")
                .or_else(|| {
                    s.tags
                        .get("handler_name")
                        .filter(|n| n.as_str() != "SoundHandler")
                })
                .cloned()
                .unwrap_or_else(|| {
                    if audio.len() == 1 {
                        "Системный звук".into()
                    } else {
                        format!("Дорожка {}", i + 1)
                    }
                }),
        })
        .collect();
    Ok(Clip {
        original_path: None,
        title: None,
        viewed: false,
        is_movie: false,
        path: path.to_path_buf(),
        duration,
        width: video.width.unwrap_or(0),
        height: video.height.unwrap_or(0),
        bytes: fs::metadata(path)?.len(),
        audio_tracks,
        audio_mix_index,
        has_audio: data
            .streams
            .iter()
            .any(|s| s.codec_type.as_deref() == Some("audio")),
        source: data
            .format
            .tags
            .get("comment")
            .and_then(|tag| tag.strip_prefix("Rewind source: "))
            .map(str::to_owned),
        game: data
            .format
            .tags
            .get("description")
            .is_some_and(|value| value == "Clipago game"),
        fps: video
            .avg_frame_rate
            .as_deref()
            .and_then(|rate| {
                let (n, d) = rate.split_once('/')?;
                let rate = n.parse::<f64>().ok()? / d.parse::<f64>().ok()?;
                (rate.is_finite() && rate > 0.0).then_some(rate)
            })
            .unwrap_or(0.0),
        aspect_ratio: display_aspect(
            video.width.unwrap_or(16),
            video.height.unwrap_or(9),
            video.sample_aspect_ratio.as_deref(),
            video
                .side_data_list
                .iter()
                .find_map(|side| side.rotation)
                .or_else(|| video.tags.get("rotate").and_then(|v| v.parse().ok()))
                .unwrap_or(0.0),
        ),
    })
}

/// Read just the foreground executable's name, not a potentially private window title.
pub fn foreground_executable() -> Option<PathBuf> {
    #[cfg(windows)]
    {
        use windows_sys::Win32::UI::WindowsAndMessaging::{
            GetForegroundWindow, GetWindowThreadProcessId,
        };
        unsafe {
            let window = GetForegroundWindow();
            if window.is_null() {
                return None;
            }
            let mut pid = 0;
            GetWindowThreadProcessId(window, &mut pid);
            (pid != std::process::id())
                .then(|| crate::audio_identity::windows::process_path(pid))
                .flatten()
        }
    }
    #[cfg(not(windows))]
    None
}

pub fn unique_path(dir: &Path, prefix: &str, extension: &str) -> PathBuf {
    let timestamp = chrono::Local::now().format("%Y-%m-%d_%H-%M-%S-%3f");
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    dir.join(format!(
        "{prefix}_{timestamp}_{:04x}.{extension}",
        nonce & 0xffff
    ))
}

pub struct WorkDir(pub PathBuf);
impl WorkDir {
    pub fn new(parent: &Path, prefix: &str) -> Result<Self> {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        fs::create_dir_all(parent)?;
        // Parallel jobs can share the same coarse Windows clock tick.
        let prefix = format!(
            "{prefix}-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        );
        let path = unique_path(parent, &prefix, "work");
        fs::create_dir(&path)?;
        Ok(Self(path))
    }
}
impl Drop for WorkDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

pub fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Publish a complete file on the same volume, atomically and without overwriting.
pub fn publish(source: &Path, destination: &Path) -> Result<()> {
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt;
        let source: Vec<u16> = source.as_os_str().encode_wide().chain(Some(0)).collect();
        let destination: Vec<u16> = destination
            .as_os_str()
            .encode_wide()
            .chain(Some(0))
            .collect();
        // With flags = 0, MoveFileExW refuses to replace an existing destination.
        if unsafe {
            windows_sys::Win32::Storage::FileSystem::MoveFileExW(
                source.as_ptr(),
                destination.as_ptr(),
                0,
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error().into());
        }
    }
    #[cfg(not(windows))]
    fs::hard_link(source, destination)?;
    Ok(())
}

pub fn run_ffmpeg(cmd: &mut Command, work: &Path, cancel: &AtomicBool) -> Result<()> {
    let log_path = work.join("ffmpeg.log");
    let log = fs::File::create(&log_path)?;
    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(log)
        .spawn()
        .context("Не удалось запустить FFmpeg")?;
    loop {
        if cancel.load(Ordering::Relaxed) {
            stop_child(&mut child);
            bail!("Операция отменена");
        }
        if let Some(status) = child.try_wait()? {
            if status.success() {
                return Ok(());
            }
            let message = fs::read_to_string(&log_path).unwrap_or_default();
            let tail = message
                .lines()
                .rev()
                .take(8)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
                .collect::<Vec<_>>()
                .join("\n");
            bail!("FFmpeg: {tail}");
        }
        thread::sleep(Duration::from_millis(80));
    }
}

#[derive(Clone, Copy, Default, Debug, PartialEq, Deserialize, Serialize)]
pub enum Quality {
    #[default]
    Bitrate,
    Target,
    Copy,
}
impl Quality {
    pub const ALL: [Self; 2] = [Self::Bitrate, Self::Target];
    pub fn label(self) -> &'static str {
        match self {
            Self::Bitrate => "Битрейт",
            Self::Target => "Размер",
            Self::Copy => "Без сжатия",
        }
    }
}

#[derive(Clone)]
pub struct Export {
    pub clip: Clip,
    pub output: PathBuf,
    pub start: f64,
    pub end: f64,
    pub quality: Quality,
    pub bitrate_mbps: f64,
    pub target_mb: f64,
    pub max_height: Option<u32>,
    pub encoding: EncodingOptions,
    pub audio_tracks: Option<Vec<u32>>,
}

/// Validate against the probed streams. The hidden mix is used only when every
/// application is selected; otherwise it would reintroduce deselected sound.
pub fn selected_audio(clip: &Clip, selection: Option<&[u32]>) -> Result<Vec<u32>> {
    let mut selected = selection
        .map(|s| s.to_vec())
        .unwrap_or_else(|| clip.audio_tracks.iter().map(|t| t.index).collect());
    selected.sort_unstable();
    selected.dedup();
    if selected
        .iter()
        .any(|i| !clip.audio_tracks.iter().any(|t| t.index == *i))
    {
        bail!("Неизвестная аудиодорожка");
    }
    if !selected.is_empty()
        && selected.len() == clip.audio_tracks.len()
        && let Some(mix) = clip.audio_mix_index
    {
        return Ok(vec![mix]);
    }
    Ok(selected)
}

#[derive(Clone, Serialize)]
pub struct PreviewAudioTrack {
    pub index: u32,
    pub path: PathBuf,
}

/// Small browser-playable stems, prepared once per source version. Rewind AAC
/// tracks are remuxed without decoding video or re-encoding audio.
pub fn prepare_audio_tracks(
    ffmpeg: &str,
    clip: &Clip,
    cache: &Path,
    cancel: &AtomicBool,
) -> Result<Vec<PreviewAudioTrack>> {
    use std::hash::{Hash, Hasher};
    let metadata = fs::metadata(&clip.path)?;
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    clip.path.hash(&mut hash);
    metadata.len().hash(&mut hash);
    metadata.modified()?.hash(&mut hash);
    let key = hash.finish();
    let tracks: Vec<_> = clip
        .audio_tracks
        .iter()
        .map(|track| PreviewAudioTrack {
            index: track.index,
            path: cache.join(format!("audio-{key:x}-{}.m4a", track.index)),
        })
        .collect();
    if tracks.is_empty() || tracks.iter().all(|t| t.path.is_file()) {
        return Ok(tracks);
    }
    let work = WorkDir::new(cache, ".audio-preview")?;
    let mut cmd = command(tool(ffmpeg));
    cmd.args(["-v", "error", "-nostdin", "-y", "-threads", "1", "-i"])
        .arg(&clip.path);
    let mut staged = Vec::new();
    for track in &clip.audio_tracks {
        let path = work.0.join(format!("{}.m4a", track.index));
        cmd.args([
            "-map",
            &format!("0:a:{}", track.index),
            "-vn",
            "-c:a",
            if track.codec == "aac" { "copy" } else { "aac" },
        ]);
        if track.codec != "aac" {
            cmd.args(["-b:a", "160k", "-af", "aresample=48000:async=1:first_pts=0"]);
        }
        cmd.args(["-movflags", "+faststart"]).arg(&path);
        staged.push(path);
    }
    run_ffmpeg(&mut cmd, &work.0, cancel)?;
    for (source, track) in staged.iter().zip(&tracks) {
        if !track.path.exists() {
            publish(source, &track.path)?;
        }
    }
    Ok(tracks)
}

fn map_audio(cmd: &mut Command, tracks: &[u32], encoder: &str, copy: bool) {
    match tracks {
        [] => {
            cmd.arg("-an");
        }
        [index] => {
            cmd.args([
                "-map",
                &format!("0:a:{index}"),
                "-c:a",
                if copy { "copy" } else { encoder },
            ]);
            if !copy {
                cmd.args(["-b:a", "128k"]);
            }
        }
        _ => {
            let inputs = tracks
                .iter()
                .map(|i| format!("[0:a:{i}]"))
                .collect::<String>();
            cmd.args(["-filter_complex_threads", "1", "-filter_complex", &format!("{inputs}amix=inputs={}:normalize=0:dropout_transition=0,alimiter=limit=0.95:level=0:latency=1[audio]", tracks.len()), "-map", "[audio]", "-c:a", encoder, "-b:a", "128k"]);
        }
    }
    if !tracks.is_empty() {
        cmd.args([
            "-metadata:s:a:0",
            "handler_name=SoundHandler",
            "-metadata:s:a:0",
            "title=Audio",
            "-disposition:a:0",
            "default",
        ]);
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum OutputFormat {
    #[default]
    Mp4,
    Mkv,
    Webm,
    Gif,
}
impl OutputFormat {
    pub const ALL: [Self; 4] = [Self::Mp4, Self::Mkv, Self::Webm, Self::Gif];
    pub fn label(self) -> &'static str {
        match self {
            Self::Mp4 => "MP4",
            Self::Mkv => "MKV",
            Self::Webm => "WebM",
            Self::Gif => "GIF",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Mp4 => "mp4",
            Self::Mkv => "mkv",
            Self::Webm => "webm",
            Self::Gif => "gif",
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
pub enum VideoCodec {
    #[default]
    H264,
    Hevc,
    Vp9,
}
impl VideoCodec {
    pub fn label(self) -> &'static str {
        match self {
            Self::H264 => "H.264 · совместимость",
            Self::Hevc => "H.265 · меньше размер",
            Self::Vp9 => "VP9",
        }
    }
}

#[derive(Clone, Copy, Default, PartialEq, Deserialize, Serialize)]
pub struct EncodingOptions {
    pub format: OutputFormat,
    pub codec: VideoCodec,
    pub fps: Option<u32>,
    pub hardware: bool,
}

/// Check real hardware availability without creating files or caching a failed probe.
pub(crate) fn hardware_encoder(
    ffmpeg: &str,
    codec: VideoCodec,
    cancel: &AtomicBool,
) -> Result<String> {
    let candidates = match codec {
        VideoCodec::H264 => ["h264_nvenc", "h264_qsv", "h264_amf"],
        VideoCodec::Hevc => ["hevc_nvenc", "hevc_qsv", "hevc_amf"],
        VideoCodec::Vp9 => bail!("Аппаратный экспорт VP9 не поддерживается"),
    };
    for encoder in candidates {
        if cancel.load(Ordering::Relaxed) {
            bail!("Операция отменена");
        }
        let mut child = command(tool(ffmpeg))
            .args([
                "-v",
                "error",
                "-nostdin",
                "-f",
                "lavfi",
                "-i",
                "color=size=160x96:rate=30,format=nv12",
                "-frames:v",
                "3",
                "-c:v",
                encoder,
                "-f",
                "null",
                if cfg!(windows) { "NUL" } else { "/dev/null" },
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        loop {
            if cancel.load(Ordering::Relaxed) || std::time::Instant::now() >= deadline {
                stop_child(&mut child);
                break;
            }
            if let Some(status) = child.try_wait()? {
                if status.success() {
                    return Ok(encoder.into());
                }
                break;
            }
            thread::sleep(Duration::from_millis(30));
        }
    }
    bail!("Аппаратный кодировщик недоступен. Отключите аппаратное ускорение для экспорта.")
}

pub fn target_video_bitrate(bytes: f64, duration: f64, audio: bool) -> Result<u64> {
    if !bytes.is_finite() || !duration.is_finite() || bytes <= 0.0 || duration <= 0.0 {
        bail!("Неверный размер или длительность");
    }
    let bitrate = bytes * 8.0 * 0.94 / duration - if audio { 128_000.0 } else { 0.0 };
    if bitrate < 64_000.0 {
        bail!("Размер слишком мал для выбранного отрезка");
    }
    Ok(bitrate as u64)
}

pub fn export(ffmpeg: &str, request: &Export, cancel: Arc<AtomicBool>) -> Result<PathBuf> {
    let audio_tracks = selected_audio(&request.clip, request.audio_tracks.as_deref())?;
    let duration = request.end - request.start;
    if request.start < 0.0
        || !duration.is_finite()
        || duration < (1.0 / request.clip.fps.max(1.0)).min(0.05) - 1e-6
        || request.end > request.clip.duration + 0.05
    {
        bail!("Выберите корректный отрезок видео");
    }
    if request.output == request.clip.path {
        bail!("Нельзя перезаписать исходный клип");
    }
    if request.output.exists() {
        bail!("Файл уже существует. Выберите другое имя.");
    }
    let options = request.encoding;
    if request.quality == Quality::Copy {
        if !matches!(options.format, OutputFormat::Mp4 | OutputFormat::Mkv)
            || request.max_height.is_some()
            || options.fps.is_some()
        {
            bail!("Без сжатия доступны MP4/MKV с исходным разрешением и FPS");
        }
        let work = WorkDir::new(
            request.output.parent().context("Нет папки экспорта")?,
            ".export",
        )?;
        let partial = work.0.join(format!("video.{}", options.format.extension()));
        let mut cmd = command(tool(ffmpeg));
        cmd.args([
            "-v",
            "error",
            "-nostdin",
            "-y",
            "-ss",
            &format!("{:.6}", request.start),
        ])
        .arg("-i")
        .arg(&request.clip.path)
        .args([
            "-t",
            &format!("{duration:.6}"),
            "-map",
            "0:v:0",
            "-c:v",
            "copy",
            "-avoid_negative_ts",
            "make_zero",
        ]);
        map_audio(&mut cmd, &audio_tracks, "aac", true);
        if options.format == OutputFormat::Mp4 {
            cmd.args(["-movflags", "+faststart"]);
        }
        cmd.arg(&partial);
        run_ffmpeg(&mut cmd, &work.0, &cancel)?;
        publish(&partial, &request.output)?;
        return Ok(request.output.clone());
    }
    let gif = options.format == OutputFormat::Gif;
    if options.hardware && (gif || options.codec == VideoCodec::Vp9) {
        bail!("Аппаратный экспорт доступен для H.264 и H.265 в MP4/MKV");
    }
    if options.fps.is_some_and(|fps| fps == 0 || fps > 240) {
        bail!("Неверная частота кадров");
    }
    if !gif && ((options.format == OutputFormat::Webm) != (options.codec == VideoCodec::Vp9)) {
        bail!("WebM требует кодек VP9; H.264 и H.265 доступны в MP4 и MKV");
    }
    if request.quality == Quality::Target
        && (gif || options.codec != VideoCodec::H264 || options.hardware)
    {
        bail!("Экспорт под размер доступен для H.264 без аппаратного ускорения");
    }
    if !gif
        && request.quality == Quality::Bitrate
        && (!request.bitrate_mbps.is_finite() || !(0.1..=1000.0).contains(&request.bitrate_mbps))
    {
        bail!("Битрейт должен быть от 0,1 до 1000 Мбит/с");
    }
    if cancel.load(Ordering::Relaxed) {
        bail!("Операция отменена");
    }
    let work = WorkDir::new(
        request.output.parent().context("Нет папки экспорта")?,
        ".export",
    )?;
    let partial = work.0.join(format!("video.{}", options.format.extension()));
    let target_rate = if request.quality == Quality::Target {
        Some(target_video_bitrate(
            request.target_mb * 1_000_000.0,
            duration,
            !audio_tracks.is_empty(),
        )?)
    } else {
        None
    };
    let scale = match request.max_height {
        Some(height) => format!("scale=-2:'min({height},ih)':force_divisible_by=2,setsar=1"),
        None => "scale=trunc(iw/2)*2:trunc(ih/2)*2,setsar=1".into(),
    };
    let fps = if gif {
        Some(options.fps.unwrap_or(15))
    } else {
        options.fps
    };
    let filter = fps.map_or(scale.clone(), |fps| format!("fps={fps},{scale}"));
    let encoder = if gif {
        String::new()
    } else if options.hardware {
        hardware_encoder(ffmpeg, options.codec, &cancel)?
    } else {
        match options.codec {
            VideoCodec::H264 => "libx264",
            VideoCodec::Hevc => "libx265",
            VideoCodec::Vp9 => "libvpx-vp9",
        }
        .into()
    };
    for pass in 1..=if target_rate.is_some() { 2 } else { 1 } {
        let mut cmd = command(tool(ffmpeg));
        cmd.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-filter_threads",
            "2",
            "-filter_complex_threads",
            "1",
            "-ss",
            &format!("{:.6}", request.start),
            "-t",
            &format!("{duration:.6}"),
        ])
        .arg("-i")
        .arg(&request.clip.path);
        if gif {
            cmd.args(["-filter_complex", &format!("[0:v:0]{filter},split[v][p];[p]palettegen=stats_mode=diff[pal];[v][pal]paletteuse=dither=bayer[gif]"),
                "-map", "[gif]", "-an", "-loop", "0"]).arg(&partial);
        } else {
            cmd.args([
                "-t",
                &format!("{duration:.6}"),
                "-map",
                "0:v:0",
                "-vf",
                &filter,
                "-c:v",
                &encoder,
                "-pix_fmt",
                "yuv420p",
            ]);
            match encoder.as_str() {
                "libx264" => {
                    cmd.args(["-preset", "veryfast", "-threads", "4"]);
                }
                "libx265" => {
                    cmd.args([
                        "-preset",
                        "veryfast",
                        "-x265-params",
                        "pools=2:frame-threads=2:log-level=error",
                    ]);
                }
                "libvpx-vp9" => {
                    cmd.args([
                        "-deadline",
                        "good",
                        "-cpu-used",
                        "4",
                        "-row-mt",
                        "1",
                        "-threads",
                        "4",
                    ]);
                }
                name if name.ends_with("_nvenc") => {
                    cmd.args(["-preset", "p4"]);
                }
                name if name.ends_with("_qsv") => {
                    cmd.args(["-preset", "veryfast"]);
                }
                name if name.ends_with("_amf") => {
                    cmd.args(["-quality", "speed"]);
                }
                _ => {}
            }
            let bits =
                target_rate.unwrap_or_else(|| (request.bitrate_mbps * 1_000_000.0).round() as u64);
            cmd.args(["-b:v", &bits.to_string()]);
            if target_rate.is_some() {
                cmd.args(["-pass", &pass.to_string(), "-passlogfile"])
                    .arg(work.0.join("pass"));
            }
            if target_rate.is_some() && pass == 1 {
                cmd.args(["-an", "-f", "null"]).arg(if cfg!(windows) {
                    "NUL"
                } else {
                    "/dev/null"
                });
            } else {
                let audio_encoder = if options.format == OutputFormat::Webm {
                    "libopus"
                } else {
                    "aac"
                };
                map_audio(&mut cmd, &audio_tracks, audio_encoder, false);
                if options.format == OutputFormat::Mp4 {
                    cmd.args(["-movflags", "+faststart"]);
                    if options.codec == VideoCodec::Hevc {
                        cmd.args(["-tag:v", "hvc1"]);
                    }
                }
                cmd.arg(&partial);
            }
        }
        run_ffmpeg(&mut cmd, &work.0, &cancel)?;
    }
    publish(&partial, &request.output).context("Не удалось сохранить экспорт")?;
    Ok(request.output.clone())
}

pub fn timecode(seconds: f64) -> String {
    let millis = (seconds.max(0.0) * 1000.0).round() as u64;
    format!(
        "{:02}:{:02}.{:01}",
        millis / 60_000,
        millis / 1000 % 60,
        millis % 1000 / 100
    )
}

pub fn size_label(bytes: u64) -> String {
    format!("{:.1} МБ", bytes as f64 / 1_000_000.0)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn size_budget_accounts_for_audio_and_container() {
        assert_eq!(
            target_video_bitrate(10_000_000.0, 60.0, true).unwrap(),
            1_125_333
        );
        assert!(target_video_bitrate(10_000.0, 120.0, true).is_err());
        assert!(target_video_bitrate(f64::NAN, 60.0, false).is_err());
    }
    #[test]
    fn time_labels_carry_at_minute_boundary() {
        assert_eq!(timecode(60.0), "01:00.0");
        assert_eq!(timecode(-1.0), "00:00.0");
    }

    #[test]
    fn publishing_refuses_existing_destination() {
        let work = WorkDir::new(&std::env::temp_dir(), "rewind-publish-test").unwrap();
        let source = work.0.join("partial.mp4");
        let destination = work.0.join("existing.mp4");
        fs::write(&source, b"new clip").unwrap();
        fs::write(&destination, b"original").unwrap();
        assert!(publish(&source, &destination).is_err());
        assert_eq!(fs::read(&destination).unwrap(), b"original");
        fs::remove_file(&destination).unwrap();
        publish(&source, &destination).unwrap();
        assert_eq!(fs::read(destination).unwrap(), b"new clip");
    }

    #[test]
    fn display_ratio_respects_pixel_shape_and_rotation() {
        assert!((display_aspect(1920, 1080, None, 0.0) - 16.0 / 9.0).abs() < 0.001);
        assert!((display_aspect(1920, 1080, None, -90.0) - 9.0 / 16.0).abs() < 0.001);
        assert!((display_aspect(720, 576, Some("16:15"), 0.0) - 4.0 / 3.0).abs() < 0.001);
    }
}
