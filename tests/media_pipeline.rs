use rewind::{
    media::{self, EncodingOptions, Export, OutputFormat, Quality, VideoCodec, WorkDir},
    recorder::{self, Event, Recorder, Request},
    replay::{REPLAY_BYTE_LIMIT, ReplayBuffer, TS_PACKET_SIZE},
    settings::{Encoder, Settings},
};
use std::{
    fs,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};

fn tools_available() -> bool {
    media::check_tools("ffmpeg", "ffprobe").is_ok()
}

fn sample(work: &WorkDir) -> std::path::PathBuf {
    let path = work.0.join("source.mp4");
    let status = media::command("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=640x360:rate=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-t",
            "5",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .status()
        .unwrap();
    assert!(status.success());
    path
}

#[test]
fn trims_compresses_and_preserves_source_and_audio() {
    if !tools_available() {
        eprintln!("FFmpeg unavailable; skipping media integration test");
        return;
    }
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-test").unwrap();
    let path = sample(&work);
    let original = fs::read(&path).unwrap();
    let clip = media::probe("ffprobe", &path).unwrap();
    let request = Export {
        clip,
        output: work.0.join("trim.mp4"),
        start: 0.73,
        end: 3.27,
        quality: Quality::Bitrate,
        bitrate_mbps: 1.5,
        target_mb: 1.0,
        audio_tracks: None,
        max_height: Some(480),
        encoding: Default::default(),
    };
    let output = media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).unwrap();
    let result = media::probe("ffprobe", &output).unwrap();
    assert!(
        (result.duration - 2.54).abs() < 0.12,
        "duration={}",
        result.duration
    );
    assert_eq!((result.width, result.height), (640, 360));
    assert!(result.has_audio);
    assert_eq!(fs::read(path).unwrap(), original);
    let saved = fs::read(output).unwrap();
    assert!(media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).is_err());
    assert_eq!(fs::read(&request.output).unwrap(), saved);
}

#[test]
fn unchanged_export_copies_the_compressed_video_payload() {
    if !tools_available() {
        return;
    }
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-copy-test").unwrap();
    let path = sample(&work);
    let clip = media::probe("ffprobe", &path).unwrap();
    let request = Export {
        clip,
        output: work.0.join("copy.mp4"),
        start: 0.0,
        end: 5.0,
        quality: Quality::Copy,
        bitrate_mbps: 1.0,
        target_mb: 25.0,
        audio_tracks: None,
        max_height: None,
        encoding: EncodingOptions::default(),
    };
    let output = media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).unwrap();
    let hash = |path: &std::path::Path| {
        let result = media::command("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-map", "0:v:0", "-c", "copy", "-f", "hash", "-hash", "md5", "-",
            ])
            .output()
            .unwrap();
        assert!(result.status.success());
        result.stdout
    };
    assert_eq!(hash(&path), hash(&output));
    assert!(media::probe("ffprobe", &output).unwrap().has_audio);
}

#[test]
fn two_pass_target_size_and_cancel_cleanup() {
    if !tools_available() {
        return;
    }
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-test").unwrap();
    let path = sample(&work);
    let clip = media::probe("ffprobe", &path).unwrap();
    let mut request = Export {
        clip,
        output: work.0.join("budget.mp4"),
        start: 0.0,
        end: 5.0,
        quality: Quality::Target,
        bitrate_mbps: 1.5,
        target_mb: 0.25,
        audio_tracks: None,
        max_height: Some(240),
        encoding: Default::default(),
    };
    let output = media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).unwrap();
    let result = media::probe("ffprobe", &output).unwrap();
    assert!(result.bytes < 250_000, "size={}", result.bytes);
    assert_eq!(result.height, 240);
    assert!(result.has_audio);
    request.output = work.0.join("cancelled.mp4");
    let cancel = Arc::new(AtomicBool::new(false));
    cancel.store(true, Ordering::Relaxed);
    assert!(media::export("ffmpeg", &request, cancel).is_err());
    assert!(!request.output.exists());
    assert!(
        !fs::read_dir(&work.0)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(".export"))
    );
}

#[test]
fn bitrate_controls_the_actual_encoded_file_size() {
    if !tools_available() {
        return;
    }
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-bitrate-test").unwrap();
    let clip = media::probe("ffprobe", &sample(&work)).unwrap();
    let mut request = Export {
        clip,
        output: work.0.join("low.mp4"),
        start: 0.0,
        end: 5.0,
        quality: Quality::Bitrate,
        bitrate_mbps: 0.4,
        target_mb: 1.0,
        audio_tracks: None,
        max_height: None,
        encoding: Default::default(),
    };
    media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).unwrap();
    let low = fs::metadata(&request.output).unwrap().len();
    request.output = work.0.join("high.mp4");
    request.bitrate_mbps = 3.0;
    media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).unwrap();
    let high = fs::metadata(&request.output).unwrap().len();
    assert!(high > low * 2, "low={low}, high={high}");
    request.output = work.0.join("invalid.mp4");
    request.bitrate_mbps = f64::NAN;
    assert!(media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).is_err());
    assert!(!request.output.exists());
}

#[test]
fn export_formats_codecs_and_frame_rates_are_real() {
    if !tools_available() {
        return;
    }
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-formats-test").unwrap();
    let clip = media::probe("ffprobe", &sample(&work)).unwrap();
    for (format, codec, expected_codec) in [
        (OutputFormat::Mp4, VideoCodec::Hevc, "hevc"),
        (OutputFormat::Mkv, VideoCodec::H264, "h264"),
        (OutputFormat::Webm, VideoCodec::Vp9, "vp9"),
        (OutputFormat::Gif, VideoCodec::H264, "gif"),
    ] {
        let output = work.0.join(format!("result.{}", format.extension()));
        let request = Export {
            clip: clip.clone(),
            output: output.clone(),
            start: 0.5,
            end: 2.5,
            quality: Quality::Bitrate,
            bitrate_mbps: 0.5,
            target_mb: 1.0,
            audio_tracks: None,
            max_height: Some(180),
            encoding: EncodingOptions {
                format,
                codec,
                fps: Some(15),
                hardware: false,
            },
        };
        media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).unwrap();
        let result = media::probe("ffprobe", &output).unwrap();
        assert_eq!((result.width, result.height), (320, 180));
        assert!(
            (result.duration - 2.0).abs() < 0.2,
            "{format:?}: {}",
            result.duration
        );
        assert_eq!(result.has_audio, format != OutputFormat::Gif);
        let probe = media::command("ffprobe")
            .args([
                "-v",
                "error",
                "-select_streams",
                "v:0",
                "-show_entries",
                "stream=codec_name,nb_read_frames",
                "-of",
                "json",
                "-count_frames",
            ])
            .arg(&output)
            .output()
            .unwrap();
        let data: serde_json::Value = serde_json::from_slice(&probe.stdout).unwrap();
        // GIF timestamps use centiseconds; the advertised rate can describe
        // its first frame delay. Verify the actual animation's frame count.
        let frames = data["streams"][0]["nb_read_frames"]
            .as_str()
            .unwrap()
            .parse::<f64>()
            .unwrap();
        assert!(
            (frames / result.duration - 15.0).abs() < 0.3,
            "{format:?}: {frames} frames / {}s",
            result.duration
        );
        if format != OutputFormat::Gif {
            assert!((result.fps - 15.0).abs() < 0.2);
        }
        assert_eq!(
            data["streams"][0]["codec_name"].as_str(),
            Some(expected_codec)
        );
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
    }
}

#[test]
#[ignore = "Requires a working NVIDIA, Intel or AMD hardware encoder"]
fn hardware_export_h264_and_hevc() {
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-gpu-export-test").unwrap();
    let clip = media::probe("ffprobe", &sample(&work)).unwrap();
    for codec in [VideoCodec::H264, VideoCodec::Hevc] {
        let request = Export {
            clip: clip.clone(),
            output: work.0.join(format!("{codec:?}.mp4")),
            start: 0.0,
            end: 1.5,
            quality: Quality::Bitrate,
            bitrate_mbps: 1.0,
            target_mb: 1.0,
            audio_tracks: None,
            max_height: Some(180),
            encoding: EncodingOptions {
                codec,
                hardware: true,
                fps: Some(30),
                ..Default::default()
            },
        };
        media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).unwrap();
        let result = media::probe("ffprobe", &request.output).unwrap();
        assert_eq!(result.height, 180);
        assert!(result.has_audio);
        assert!((result.duration - 1.5).abs() < 0.1);
    }
}

#[test]
fn ram_replay_retains_decodable_tail_and_immutable_snapshot() {
    if !tools_available() {
        return;
    }
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-ram-test").unwrap();
    let stream = media::command("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30",
            "-t",
            "12",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "60",
            "-bf",
            "0",
            "-f",
            "mpegts",
            "-muxdelay",
            "0",
            "-flush_packets",
            "1",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(
        stream.status.success(),
        "{}",
        String::from_utf8_lossy(&stream.stderr)
    );
    assert_eq!(stream.stdout.len() % TS_PACKET_SIZE, 0);
    let mut ring = ReplayBuffer::new(3, 30, REPLAY_BYTE_LIMIT);
    let mut early = None;
    for packet in stream.stdout.chunks_exact(TS_PACKET_SIZE) {
        ring.push_packet(packet.try_into().unwrap()).unwrap();
        if early.is_none() && ring.duration() >= 3.4 {
            early = Some(ring.snapshot().unwrap());
        }
        assert!(ring.buffered_bytes() <= REPLAY_BYTE_LIMIT);
    }
    ring.finish_stream();
    assert!(
        fs::read_dir(&work.0).unwrap().next().is_none(),
        "Buffering must not create files"
    );
    let snapshot = ring.snapshot().unwrap();
    assert!(snapshot.duration >= 1.0 && snapshot.duration <= 3.0);
    for (name, mut snapshot) in [("early.mp4", early.unwrap()), ("tail.mp4", snapshot)] {
        snapshot.source = Some("Test Game".into());
        let expected = snapshot.duration;
        let path = work.0.join(name);
        recorder::save_snapshot("ffmpeg", snapshot, &path, Arc::new(AtomicBool::new(false)))
            .unwrap();
        let clip = media::probe("ffprobe", &path).unwrap();
        assert_eq!(clip.source.as_deref(), Some("Test Game"));
        assert!(
            (clip.duration - expected).abs() < 0.1,
            "expected={expected} got={}",
            clip.duration
        );
        let decoded = media::command("ffmpeg")
            .args(["-v", "error", "-xerror", "-i"])
            .arg(&path)
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
        let content = fs::read(&path).unwrap();
        assert!(
            recorder::save_snapshot(
                "ffmpeg",
                ring.snapshot().unwrap(),
                &path,
                Arc::new(AtomicBool::new(false))
            )
            .is_err()
        );
        assert_eq!(fs::read(path).unwrap(), content);
    }
    let cancelled = work.0.join("cancelled.mp4");
    assert!(
        recorder::save_snapshot(
            "ffmpeg",
            ring.snapshot().unwrap(),
            &cancelled,
            Arc::new(AtomicBool::new(true))
        )
        .is_err()
    );
    assert!(!cancelled.exists());
    assert!(
        !fs::read_dir(&work.0)
            .unwrap()
            .flatten()
            .any(|e| e.path().is_dir())
    );
}

#[test]
#[ignore = "Captures this Windows desktop for a few seconds; run explicitly"]
fn windows_desktop_replay_smoke() {
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-desktop-test").unwrap();
    let recorder = Recorder::new(Arc::new(|| {}));
    let settings = Settings {
        output_dir: work.0.join("clips"),
        encoder: Encoder::Auto,
        buffer_seconds: 4,
        ..Settings::default()
    };
    let output_dir = settings.output_dir.clone();
    recorder.tx.send(Request::Start(settings)).unwrap();
    let deadline = Instant::now() + Duration::from_secs(50);
    let mut requested = false;
    let mut saved = None;
    let mut encoder = String::new();
    let mut saved_once = false;
    while Instant::now() < deadline {
        match recorder.rx.recv_timeout(Duration::from_secs(1)) {
            Ok(Event::Started(name)) => encoder = name,
            Ok(Event::Buffer(seconds)) if seconds >= 3.0 && !requested => {
                assert!(!output_dir.exists(), "Capture created files before Save");
                requested = true;
                recorder.tx.send(Request::Save).unwrap();
            }
            Ok(Event::Saved(result)) => {
                let path = result.unwrap();
                let clip = media::probe("ffprobe", &path).unwrap();
                assert!(
                    clip.has_audio,
                    "System-audio recording must produce an audio track"
                );
                let decoded = media::command("ffmpeg")
                    .args(["-v", "error", "-xerror", "-i"])
                    .arg(&path)
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
                assert!(
                    clip.duration > 0.0 && clip.duration <= 4.05,
                    "{}",
                    clip.duration
                );
                if saved_once {
                    saved = Some(path);
                    break;
                }
                saved_once = true;
                std::thread::sleep(Duration::from_secs(5));
                recorder.tx.send(Request::Save).unwrap();
            }
            Ok(Event::Error(error)) => panic!("{error}"),
            Ok(Event::CaptureFailed { message, .. }) => panic!("{message}"),
            _ => {}
        }
    }
    recorder.tx.send(Request::Stop).unwrap();
    let path = saved.expect("Timed out saving desktop replay");
    let clip = media::probe("ffprobe", &path).unwrap();
    assert!(clip.duration > 0.0 && clip.duration <= 4.05);
    assert!(clip.width > 0 && clip.height > 0);
    eprintln!(
        "Captured with {encoder}: {}x{}, {:.2}s",
        clip.width, clip.height, clip.duration
    );
    drop(recorder);
}

#[test]
fn sparse_video_keyframes_follow_time_and_saved_replay_has_a_hard_limit() {
    if !tools_available() {
        return;
    }
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-sparse-window").unwrap();
    let stream = media::command("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "testsrc2=size=320x180:rate=30",
            "-frames:v",
            "180",
            "-vf",
            "setpts='PTS+if(gte(N,60),20/TB,0)'",
            "-fps_mode",
            "passthrough",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "120",
            "-bf",
            "0",
            "-force_key_frames",
            "expr:if(isnan(prev_forced_t),1,gte(t,prev_forced_t+2))",
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
    let mut ring = ReplayBuffer::new(10, 30, REPLAY_BYTE_LIMIT);
    for packet in stream.stdout.chunks_exact(TS_PACKET_SIZE) {
        ring.push_packet(packet.try_into().unwrap()).unwrap();
    }
    ring.finish_stream();
    let snapshot = ring.snapshot().unwrap();
    // There must be an IDR immediately after the 20-second timestamp gap,
    // instead of waiting for the 120-frame GOP counter to reach four seconds.
    assert!(
        (snapshot.duration - 10.0).abs() < 0.001,
        "{}",
        snapshot.duration
    );
    let output = work.0.join("tail.mp4");
    recorder::save_snapshot(
        "ffmpeg",
        snapshot,
        &output,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let clip = media::probe("ffprobe", &output).unwrap();
    assert!((clip.duration - 10.0).abs() < 0.001);
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
}
