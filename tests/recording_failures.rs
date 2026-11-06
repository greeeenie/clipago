#![cfg(windows)]

use rewind::{
    media::WorkDir,
    recorder::{Event, Recorder, Request},
    settings::Settings,
};
use std::{fs, sync::Arc, time::Duration};

fn failed_capture(message: &str, temporary: bool, attempts: usize) {
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-capture-failure").unwrap();
    let ffmpeg = work.0.join("ffmpeg.cmd");
    fs::write(
        &ffmpeg,
        format!(
            "@echo off\r\nif \"%3\"==\"filter=gfxcapture\" exit /b 0\r\n>>\"%~dp0calls.txt\" echo capture\r\n>&2 echo {message}\r\nexit /b 1\r\n"
        ),
    )
    .unwrap();
    let recorder = Recorder::new(Arc::new(|| {}));
    let output_dir = work.0.join("clips");
    recorder
        .tx
        .send(Request::Start(Settings {
            ffmpeg: ffmpeg.to_string_lossy().into_owned(),
            output_dir: output_dir.clone(),
            system_audio: false,
            ..Settings::default()
        }))
        .unwrap();
    match recorder.rx.recv_timeout(Duration::from_secs(10)).unwrap() {
        Event::CaptureFailed {
            message: actual,
            temporary: actual_temporary,
        } => {
            assert_eq!(actual_temporary, temporary);
            assert!(actual.contains(message));
        }
        _ => panic!("Expected a typed capture failure"),
    }
    assert_eq!(
        fs::read_to_string(work.0.join("calls.txt"))
            .unwrap()
            .lines()
            .count(),
        attempts
    );
    assert!(
        !output_dir.exists(),
        "A capture failure must not create video files"
    );
    drop(recorder);
}

#[test]
fn lost_desktop_does_not_fall_back_through_encoders_or_cpu() {
    failed_capture(
        "[Parsed_ddagrab_0] AcquireNextFrame failed: 887A0026",
        true,
        1,
    );
}

#[test]
fn encoder_failure_keeps_auto_fallback_and_remains_visible() {
    failed_capture("Unknown encoder", false, 5);
}

#[test]
fn idle_live_source_is_kept_instead_of_switching_to_cpu() {
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-idle-source").unwrap();
    let source = work.0.join("idle.rs");
    let executable = work.0.join("idle.exe");
    fs::write(&source,r#"fn main() {
        if std::env::args().any(|arg|arg=="filter=gfxcapture") {return;}
        let path=std::env::current_exe().unwrap().with_extension("calls");
        use std::io::Write;
        writeln!(std::fs::OpenOptions::new().create(true).append(true).open(path).unwrap(),"capture").unwrap();
        std::thread::sleep(std::time::Duration::from_secs(10));
    }"#).unwrap();
    let compiled = rewind::media::command("rustc")
        .arg(&source)
        .arg("-o")
        .arg(&executable)
        .output()
        .unwrap();
    assert!(
        compiled.status.success(),
        "{}",
        String::from_utf8_lossy(&compiled.stderr)
    );
    let recorder = Recorder::new(Arc::new(|| {}));
    recorder
        .tx
        .send(Request::Start(Settings {
            ffmpeg: executable.to_string_lossy().into_owned(),
            system_audio: false,
            ..Default::default()
        }))
        .unwrap();
    match recorder.rx.recv_timeout(Duration::from_secs(5)).unwrap() {
        Event::Started(encoder) => assert_eq!(encoder, "NVIDIA NVENC"),
        _ => panic!("A healthy idle source should remain active"),
    }
    assert_eq!(
        fs::read_to_string(executable.with_extension("calls"))
            .unwrap()
            .lines()
            .count(),
        1
    );
    drop(recorder);
}

#[test]
fn replay_survives_capture_loss_and_saves_both_sides_of_recovery() {
    recovery_fixture(false);
}

#[test]
#[ignore = "Captures Windows sound and plays a quiet tone during simulated video loss"]
fn system_audio_continues_while_video_capture_is_recovering() {
    recovery_fixture(true);
}

fn recovery_fixture(audio_on: bool) {
    use rewind::media;
    use std::{path::PathBuf, time::Instant};
    let Some(real_ffmpeg) = std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|path| path.join("ffmpeg.exe"))
            .find(|path| path.is_file())
    }) else {
        return;
    };
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-capture-recovery").unwrap();
    let ffmpeg = work.0.join("ffmpeg.cmd");
    // Produce real, independently encoded red/blue streams whose clocks both
    // start at zero, then simulate the exact DXGI error from the user's capture.
    fs::write(&ffmpeg, format!(
        "@echo off\r\nif \"%3\"==\"filter=gfxcapture\" exit /b 0\r\nif not \"%3\"==\"warning\" goto remux\r\nset replay_color=red\r\nif exist \"%~dp0second\" set replay_color=blue\r\n\"{real}\" -v error -re -f lavfi -i color=c=%replay_color%:s=320x180:r=30 -t 2 -c:v libx264 -preset ultrafast -g 30 -bf 0 -f mpegts -mpegts_start_pid 256 -mpegts_pmt_start_pid 4096 -muxdelay 0 -flush_packets 1 pipe:1\r\n>&2 echo [Parsed_ddagrab_0] AcquireNextFrame failed: 887A0026\r\nexit /b 1\r\n:remux\r\n\"{real}\" %*\r\nexit /b %errorlevel%\r\n", real=real_ffmpeg.display()
    )).unwrap();
    let settings = Settings {
        ffmpeg: ffmpeg.to_string_lossy().into_owned(),
        output_dir: work.0.join("clips"),
        encoder: rewind::settings::Encoder::Cpu,
        system_audio: audio_on,
        buffer_seconds: if audio_on { 6 } else { 3 },
        ..Settings::default()
    };
    let recorder = Recorder::new(Arc::new(|| {}));
    let wait_failure = || {
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match recorder.rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Event::CaptureFailed { temporary, .. }) => {
                    assert!(temporary);
                    return;
                }
                Ok(Event::Error(message)) => panic!("{message}"),
                Ok(Event::AudioState(Err(message))) => panic!("Audio: {message}"),
                _ => {}
            }
        }
        panic!("Timed out waiting for simulated capture loss");
    };
    let save = || -> PathBuf {
        recorder.tx.send(Request::Save).unwrap();
        let deadline = Instant::now() + Duration::from_secs(10);
        while Instant::now() < deadline {
            match recorder.rx.recv_timeout(Duration::from_secs(1)) {
                Ok(Event::Saved(result)) => return result.unwrap(),
                Ok(Event::Error(message)) => panic!("{message}"),
                _ => {}
            }
        }
        panic!("Timed out saving replay while capture was unavailable");
    };
    recorder.tx.send(Request::Start(settings.clone())).unwrap();
    wait_failure();
    assert!(!settings.output_dir.exists());
    let paused = save();
    let old_clip = media::probe("ffprobe", &paused).unwrap();
    assert!(old_clip.duration > 1.8);
    assert_eq!(old_clip.has_audio, audio_on);
    if audio_on {
        let status = media::command("ffplay")
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
    }
    fs::write(work.0.join("second"), []).unwrap();
    recorder.tx.send(Request::Start(settings)).unwrap();
    wait_failure();
    let resumed = save();
    let clip = media::probe("ffprobe", &resumed).unwrap();
    assert!(
        (clip.duration - if audio_on { 6.0 } else { 3.0 }).abs() < 0.002,
        "{}",
        clip.duration
    );
    let decoded = media::command(&real_ffmpeg)
        .args(["-v", "error", "-xerror", "-i"])
        .arg(&resumed)
        .args(["-f", "null", "NUL"])
        .output()
        .unwrap();
    assert!(
        decoded.status.success(),
        "{}",
        String::from_utf8_lossy(&decoded.stderr)
    );
    assert_eq!(clip.has_audio, audio_on);
    if audio_on {
        let sound = media::command(&real_ffmpeg)
            .args(["-v", "error", "-ss", "2", "-i"])
            .arg(&resumed)
            .args([
                "-t", "1", "-map", "0:a:0", "-ac", "1", "-f", "f32le", "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(sound.status.success());
        assert!(
            sound
                .stdout
                .chunks_exact(4)
                .any(|sample| f32::from_le_bytes(sample.try_into().unwrap()).abs() > 0.00001),
            "Sound captured during video loss was discarded"
        );
    }
    for (seconds, channel) in [(0.4, 0), (clip.duration - 0.4, 2)] {
        let pixel = media::command(&real_ffmpeg)
            .args(["-v", "error", "-ss", &seconds.to_string(), "-i"])
            .arg(&resumed)
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
        assert!(
            pixel.stdout[channel] > 200 && pixel.stdout[2 - channel] < 30,
            "Expected old red and new blue frames; got {:?}",
            pixel.stdout
        );
    }
    drop(recorder);
}
