use rewind::media::{self, EncodingOptions, Export, Quality, WorkDir};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};

fn spectrum(path: &Path) -> (f64, f64) {
    let decoded = media::command("ffmpeg")
        .args(["-v", "error", "-i"])
        .arg(path)
        .args([
            "-ss", "0.4", "-t", "1", "-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le",
            "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    let samples: Vec<f64> = decoded
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
    (power(440.0), power(880.0))
}
#[test]
fn application_track_selection_mutes_discord_in_copy_and_compressed_exports() {
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-audio-selection").unwrap();
    let source = work.0.join("apps.mp4");
    let result = media::command("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=s=160x90:r=30",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=1320:sample_rate=48000",
            "-filter_complex",
            "[1:a][2:a][3:a]amix=inputs=3:normalize=0[mix]",
            "-map",
            "0:v",
            "-map",
            "[mix]",
            "-map",
            "1:a",
            "-map",
            "2:a",
            "-map",
            "3:a",
            "-t",
            "3",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "aac",
            "-metadata:s:a:0",
            "handler_name=Rewind mix",
            "-metadata:s:a:1",
            "handler_name=Deadlock",
            "-metadata:s:a:2",
            "handler_name=Discord",
            "-metadata:s:a:3",
            "handler_name=Chrome",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let clip = media::probe("ffprobe", &source).unwrap();
    assert_eq!(clip.audio_mix_index, Some(0));
    // The preview uses only application stems, never the hidden original mix.
    // AAC payloads are copied, keeping timing and avoiding another encode.
    let previews =
        media::prepare_audio_tracks("ffmpeg", &clip, &work.0, &AtomicBool::new(false)).unwrap();
    assert_eq!(
        previews.iter().map(|t| t.index).collect::<Vec<_>>(),
        [1, 2, 3]
    );
    let payload_hash = |path: &Path, index: u32| {
        let result = media::command("ffmpeg")
            .args(["-v", "error", "-i"])
            .arg(path)
            .args([
                "-map",
                &format!("0:a:{index}"),
                "-c:a",
                "copy",
                "-f",
                "hash",
                "-hash",
                "sha256",
                "pipe:1",
            ])
            .output()
            .unwrap();
        assert!(result.status.success());
        result.stdout
    };
    for preview in &previews {
        assert_eq!(
            payload_hash(&source, preview.index),
            payload_hash(&preview.path, 0)
        );
    }
    let cached =
        media::prepare_audio_tracks("not-a-real-ffmpeg", &clip, &work.0, &AtomicBool::new(false))
            .unwrap();
    assert_eq!(
        previews.iter().map(|t| &t.path).collect::<Vec<_>>(),
        cached.iter().map(|t| &t.path).collect::<Vec<_>>()
    );
    assert_eq!(
        clip.audio_tracks
            .iter()
            .map(|t| t.name.as_str())
            .collect::<Vec<_>>(),
        ["Deadlock", "Discord", "Chrome"]
    );
    for quality in [Quality::Copy, Quality::Bitrate, Quality::Target] {
        for selection in [vec![1], vec![2], vec![3], vec![1, 2], vec![1, 2, 3], vec![]] {
            let output = work.0.join(format!("{quality:?}-{selection:?}.mp4"));
            let request = Export {
                clip: clip.clone(),
                output: output.clone(),
                start: 0.0,
                end: 3.0,
                quality,
                bitrate_mbps: 0.5,
                target_mb: 0.3,
                max_height: None,
                encoding: EncodingOptions::default(),
                audio_tracks: Some(selection.clone()),
            };
            media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).unwrap();
            let saved = media::probe("ffprobe", &output).unwrap();
            assert_eq!(saved.has_audio, !selection.is_empty());
            assert_eq!(saved.audio_tracks.len(), usize::from(!selection.is_empty()));
            if !selection.is_empty() {
                let (game, discord) = spectrum(&output);
                if selection.contains(&1) {
                    assert!(game > 0.01, "{quality:?} {selection:?}: game={game}");
                } else {
                    assert!(game < 0.0001);
                }
                if selection.contains(&2) {
                    assert!(discord > 0.01);
                } else {
                    assert!(discord < 0.0001, "Discord leaked: {discord}");
                }
            }
        }
    }
    let request = Export {
        clip,
        output: work.0.join("invalid.mp4"),
        start: 0.0,
        end: 3.0,
        quality: Quality::Copy,
        bitrate_mbps: 1.0,
        target_mb: 1.0,
        max_height: None,
        encoding: EncodingOptions::default(),
        audio_tracks: Some(vec![999]),
    };
    assert!(media::export("ffmpeg", &request, Arc::new(AtomicBool::new(false))).is_err());
}
