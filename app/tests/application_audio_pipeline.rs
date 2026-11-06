#![cfg(windows)]
use rewind::{
    audio::AudioRecorder,
    media,
    recorder::{self, SaveOptions},
    replay::{REPLAY_BYTE_LIMIT, ReplayBuffer, TS_PACKET_SIZE},
    settings::Encoder,
};
use std::{
    sync::{Arc, atomic::AtomicBool},
    time::{Duration, Instant},
};
struct Player(std::process::Child);
impl Drop for Player {
    fn drop(&mut self) {
        media::stop_child(&mut self.0);
    }
}
#[test]
#[ignore = "plays quiet tones and saves a real replay with native application audio"]
fn native_application_audio_survives_replay_encoding_as_separate_named_tracks() {
    let play = |hz| {
        Player(
            media::command("ffplay")
                .args([
                    "-nodisp",
                    "-autoexit",
                    "-f",
                    "lavfi",
                    "-i",
                    &format!("sine=frequency={hz}:sample_rate=48000:duration=8,volume=0.15"),
                ])
                .stdout(std::process::Stdio::null())
                .stderr(std::process::Stdio::null())
                .spawn()
                .unwrap(),
        )
    };
    let first = play(600);
    let second = play(1200);
    let game = AudioRecorder::for_process(first.0.id()).unwrap();
    let discord = AudioRecorder::for_process(second.0.id()).unwrap();
    game.events
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    discord
        .events
        .recv_timeout(Duration::from_secs(5))
        .unwrap()
        .unwrap();
    std::thread::sleep(Duration::from_millis(1500));
    let end = Instant::now() - Duration::from_millis(100);
    let tracks = vec![
        ("Deadlock".into(), game.snapshot(end, 1.0)),
        ("Discord".into(), discord.snapshot(end, 1.0)),
    ];
    drop(first);
    drop(second);
    let stream = media::command("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=s=160x90:r=30",
            "-t",
            "2",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-g",
            "30",
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
    assert!(stream.status.success());
    let mut ring = ReplayBuffer::new(1, 30, REPLAY_BYTE_LIMIT);
    for packet in stream.stdout.chunks_exact(TS_PACKET_SIZE) {
        ring.push_packet(packet.try_into().unwrap()).unwrap();
    }
    ring.finish_stream();
    let mut snapshot = ring.snapshot().unwrap();
    snapshot.end_time = end;
    let work = media::WorkDir::new(&std::env::temp_dir(), "rewind-app-audio-pipeline").unwrap();
    let output = work.0.join("replay.mp4");
    recorder::save_snapshot_with_options(
        "ffmpeg",
        snapshot,
        &output,
        Arc::new(AtomicBool::new(false)),
        SaveOptions {
            encoder: Encoder::Cpu,
            audio: None,
            tracks,
        },
    )
    .unwrap();
    let clip = media::probe("ffprobe", &output).unwrap();
    assert!((clip.duration - 1.0).abs() < 0.002);
    assert_eq!(clip.audio_mix_index, Some(0));
    assert_eq!(
        clip.audio_tracks
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["Deadlock", "Discord"]
    );
    for (index, wanted, absent) in [(1, 600.0, 1200.0), (2, 1200.0, 600.0)] {
        let pcm = media::command("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(&output)
            .args([
                "-ss",
                "0.2",
                "-t",
                "0.5",
                "-map",
                &format!("0:a:{index}"),
                "-ac",
                "1",
                "-ar",
                "48000",
                "-f",
                "f32le",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(pcm.status.success());
        let samples: Vec<f64> = pcm
            .stdout
            .chunks_exact(4)
            .map(|v| f32::from_le_bytes(v.try_into().unwrap()) as f64)
            .collect();
        let power = |hz: f64| {
            let (mut a, mut b) = (0.0, 0.0);
            for (i, s) in samples.iter().enumerate() {
                let phase = std::f64::consts::TAU * hz * i as f64 / 48000.0;
                a += s * phase.sin();
                b += s * phase.cos();
            }
            (a * a + b * b).sqrt() / samples.len() as f64
        };
        assert!(power(wanted) > 0.001, "missing own application audio");
        assert!(
            power(absent) < 0.0001,
            "application audio leaked during replay encoding"
        );
    }
}
