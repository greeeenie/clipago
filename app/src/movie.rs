use crate::media::{self, Clip, Export, OutputFormat, Quality, VideoCodec, WorkDir};
use anyhow::{Context, Result, ensure};
use std::{
    path::PathBuf,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
};

pub struct Part {
    pub edit: Export,
    pub preprocess: bool,
    pub gap_before: f64,
}
struct Input {
    clip: Clip,
    start: f64,
    duration: f64,
    audio: Vec<u32>,
    height: Option<u32>,
    fps: Option<u32>,
    gap_before: f64,
}

/// References and edits are resolved only at render time; source files are
/// never rewritten. Ordinary cuts/audio selections need one final encode.
pub fn render(
    ffmpeg: &str,
    ffprobe: &str,
    parts: &[Part],
    output: &Export,
    cancel: Arc<AtomicBool>,
) -> Result<PathBuf> {
    ensure!(
        !parts.is_empty() && parts.len() <= 128,
        "Добавьте от 1 до 128 фрагментов"
    );
    ensure!(!output.output.exists(), "Файл уже существует");
    ensure!(
        parts.iter().all(|p| p.edit.clip.path != output.output),
        "Нельзя перезаписать исходный клип"
    );
    let options = output.encoding;
    ensure!(
        output.quality != Quality::Copy,
        "Для мувика требуется кодирование"
    );
    let fps = options
        .fps
        .unwrap_or(
            parts
                .iter()
                .map(|p| p.edit.clip.fps.round() as u32)
                .max()
                .unwrap_or(30),
        )
        .clamp(1, 240);
    let height = (output
        .max_height
        .unwrap_or(output.clip.height)
        .min(output.clip.height)
        .clamp(90, 2160)
        / 2)
        * 2;
    let width = ((height as f64 * 16.0 / 9.0 / 2.0).round() as u32) * 2;
    ensure!(
        output.bitrate_mbps.is_finite() && (0.1..=1000.0).contains(&output.bitrate_mbps),
        "Некорректный битрейт"
    );
    ensure!(
        !matches!(options.format, OutputFormat::Webm) || options.codec == VideoCodec::Vp9,
        "WebM требует VP9"
    );
    ensure!(
        options.codec != VideoCodec::Vp9 || options.format == OutputFormat::Webm,
        "VP9 требует WebM"
    );
    let work = WorkDir::new(
        output.output.parent().context("Нет папки экспорта")?,
        ".movie",
    )?;
    let mut inputs = Vec::new();
    for (index, part) in parts.iter().enumerate() {
        ensure!(!cancel.load(Ordering::Relaxed), "Операция отменена");
        let edit = &part.edit;
        ensure!(
            part.gap_before.is_finite() && part.gap_before >= 0.0,
            "Некорректный промежуток перед фрагментом {}",
            index + 1
        );
        let gap_before = (part.gap_before * fps as f64).round() / fps as f64;
        ensure!(gap_before.is_finite(), "Промежуток слишком большой");
        ensure!(
            edit.start.is_finite()
                && edit.end.is_finite()
                && edit.start >= 0.0
                && edit.end <= edit.clip.duration + 0.05
                && edit.end - edit.start >= (1.0 / edit.clip.fps.max(1.0)).min(0.05) - 1e-6,
            "Некорректные границы фрагмента {}",
            index + 1
        );
        let selected = media::selected_audio(&edit.clip, edit.audio_tracks.as_deref())?;
        if part.preprocess {
            let mut request = edit.clone();
            request.output = work.0.join(format!("part-{index}.mp4"));
            request.encoding.format = OutputFormat::Mp4;
            request.encoding.codec = VideoCodec::H264;
            if request.quality == Quality::Copy {
                request.quality = Quality::Bitrate;
            }
            media::export(ffmpeg, &request, cancel.clone())?;
            let clip = media::probe(ffprobe, &request.output)?;
            let audio = if clip.has_audio { vec![0] } else { vec![] };
            inputs.push(Input {
                duration: ((edit.end - edit.start) * fps as f64).round().max(1.0) / fps as f64,
                clip,
                start: 0.0,
                audio,
                height: None,
                fps: None,
                gap_before,
            });
        } else {
            inputs.push(Input {
                clip: edit.clip.clone(),
                start: edit.start,
                duration: ((edit.end - edit.start) * fps as f64).round().max(1.0) / fps as f64,
                audio: selected,
                height: edit.max_height,
                fps: edit.encoding.fps,
                gap_before,
            });
        }
    }
    let duration: f64 = inputs.iter().map(|i| i.duration + i.gap_before).sum();
    ensure!(duration.is_finite(), "Длительность монтажа слишком большая");
    let gif = options.format == OutputFormat::Gif;
    let audio = !gif && inputs.iter().any(|i| !i.audio.is_empty());
    let target = if output.quality == Quality::Target {
        let max_bytes = output.target_mb * 1e6;
        Some((
            media::target_video_bitrate(max_bytes, duration, audio)?,
            max_bytes,
        ))
    } else {
        None
    };
    ensure!(
        target.is_none() || (!gif && options.codec == VideoCodec::H264 && !options.hardware),
        "Размер файла доступен для H.264 на CPU"
    );
    let encoder = if gif {
        String::new()
    } else if options.hardware {
        media::hardware_encoder(ffmpeg, options.codec, &cancel)?
    } else {
        match options.codec {
            VideoCodec::H264 => "libx264",
            VideoCodec::Hevc => "libx265",
            VideoCodec::Vp9 => "libvpx-vp9",
        }
        .into()
    };
    let partial = work.0.join(format!("movie.{}", options.format.extension()));
    media::encode_with_size_limit(&partial, target, |pass, bitrate| {
        let sound = audio && !(bitrate.is_some() && pass == 1);
        let mut command = media::command(media::tool(ffmpeg));
        command.args([
            "-hide_banner",
            "-loglevel",
            "error",
            "-nostdin",
            "-y",
            "-filter_complex_threads",
            "1",
        ]);
        for input in &inputs {
            command
                .args([
                    "-threads",
                    "2",
                    "-ss",
                    &format!("{:.9}", input.start),
                    "-t",
                    &format!("{:.9}", input.duration),
                    "-i",
                ])
                .arg(&input.clip.path);
        }
        let mut filters = Vec::new();
        let mut streams = String::new();
        let mut count = 0;
        for (index, input) in inputs.iter().enumerate() {
            if input.gap_before > 0.0 {
                filters.push(format!("color=c=black:s={width}x{height}:r={fps}:d={:.9},setsar=1,format=yuv420p,setpts=PTS-STARTPTS[gv{index}]", input.gap_before));
                streams.push_str(&format!("[gv{index}]"));
                if sound {
                    filters.push(format!("anullsrc=r=48000:cl=stereo,atrim=duration={:.9},asetpts=PTS-STARTPTS[ga{index}]", input.gap_before));
                    streams.push_str(&format!("[ga{index}]"));
                }
                count += 1;
            }
            let downscale = input
                .height
                .map(|h| format!("scale=-2:'min({h},ih)',"))
                .unwrap_or_default();
            let source_fps = input.fps.map(|f| format!("fps={f},")).unwrap_or_default();
            filters.push(format!("[{index}:v:0]setpts=PTS-STARTPTS,{downscale}{source_fps}fps={fps},scale={width}:{height}:force_original_aspect_ratio=decrease,pad={width}:{height}:(ow-iw)/2:(oh-ih)/2,setsar=1,format=yuv420p,tpad=stop_mode=clone:stop_duration=1,trim=duration={:.9},setpts=PTS-STARTPTS[v{index}]",input.duration));
            streams.push_str(&format!("[v{index}]"));
            if sound {
                let source = match input.audio.as_slice() {
                    [] => "anullsrc=r=48000:cl=stereo,".to_owned(),
                    [track] => format!("[{index}:a:{track}]"),
                    tracks => format!(
                        "{}amix=inputs={}:normalize=0:dropout_transition=0,alimiter=limit=0.95:level=0:latency=1,",
                        tracks
                            .iter()
                            .map(|t| format!("[{index}:a:{t}]"))
                            .collect::<String>(),
                        tracks.len()
                    ),
                };
                filters.push(format!("{source}aresample=48000:async=1:first_pts=0,aformat=sample_fmts=fltp:channel_layouts=stereo,apad,atrim=duration={:.9},asetpts=PTS-STARTPTS[a{index}]",input.duration));
                streams.push_str(&format!("[a{index}]"));
            }
            count += 1;
        }
        filters.push(format!(
            "{streams}concat=n={}:v=1:a={}[v]{}",
            count,
            u8::from(sound),
            if sound { "[a]" } else { "" }
        ));
        if gif {
            filters.push("[v]split[vp][pg];[pg]palettegen[pal];[vp][pal]paletteuse[gif]".into());
        }
        command.args([
            "-filter_complex",
            &filters.join(";"),
            "-map",
            if gif { "[gif]" } else { "[v]" },
            "-t",
            &format!("{duration:.9}"),
        ]);
        if gif {
            command.args(["-an", "-loop", "0"]).arg(&partial);
        } else {
            command.args([
                "-c:v",
                &encoder,
                "-pix_fmt",
                "yuv420p",
                "-b:v",
                &bitrate
                    .unwrap_or((output.bitrate_mbps * 1e6) as u64)
                    .to_string(),
            ]);
            match encoder.as_str() {
                "libx264" => {
                    command.args(["-preset", "veryfast", "-threads", "2"]);
                }
                "libx265" => {
                    command.args([
                        "-preset",
                        "veryfast",
                        "-x265-params",
                        "pools=2:frame-threads=2:log-level=error",
                    ]);
                }
                "libvpx-vp9" => {
                    command.args(["-cpu-used", "4", "-row-mt", "1", "-threads", "2"]);
                }
                s if s.ends_with("_nvenc") => {
                    command.args(["-preset", "p4"]);
                }
                _ => {}
            }
            if bitrate.is_some() {
                command
                    .args(["-pass", &pass.to_string(), "-passlogfile"])
                    .arg(work.0.join("pass"));
            }
            if bitrate.is_some() && pass == 1 {
                command.args(["-an", "-f", "null"]).arg(if cfg!(windows) {
                    "NUL"
                } else {
                    "/dev/null"
                });
            } else {
                if sound {
                    command.args([
                        "-map",
                        "[a]",
                        "-c:a",
                        if options.format == OutputFormat::Webm {
                            "libopus"
                        } else {
                            "aac"
                        },
                        "-b:a",
                        "128k",
                    ]);
                } else {
                    command.arg("-an");
                }
                if options.format == OutputFormat::Mp4 {
                    command.args(["-movflags", "+faststart"]);
                    if options.codec == VideoCodec::Hevc {
                        command.args(["-tag:v", "hvc1"]);
                    }
                }
                command
                    .args(["-metadata", "comment=Rewind movie"])
                    .arg(&partial);
            }
        }
        media::run_ffmpeg(&mut command, &work.0, &cancel)
    })?;
    ensure!(!cancel.load(Ordering::Relaxed), "Операция отменена");
    media::publish(&partial, &output.output)?;
    Ok(output.output.clone())
}
