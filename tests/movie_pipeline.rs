use rewind::{
    media::{self, Clip, EncodingOptions, Export, OutputFormat, Quality, VideoCodec, WorkDir},
    movie::{self, Part},
};
use std::{
    path::Path,
    sync::{Arc, atomic::AtomicBool},
};
fn source(work: &WorkDir, name: &str, audio: bool) -> Clip {
    let path = work.0.join(format!("{name}.mp4"));
    let mut cmd = media::command("ffmpeg");
    cmd.args([
        "-v",
        "error",
        "-f",
        "lavfi",
        "-i",
        if audio {
            "color=c=red:s=160x90:r=30:d=3"
        } else {
            "color=c=blue:s=320x180:r=60:d=3"
        },
    ]);
    if audio {
        cmd.args([
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=3",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=880:sample_rate=48000:duration=3",
            "-filter_complex",
            "[1:a][2:a]amix=inputs=2:normalize=0[mix]",
            "-map",
            "0:v",
            "-map",
            "[mix]",
            "-map",
            "1:a",
            "-map",
            "2:a",
            "-c:a",
            "aac",
            "-metadata:s:a:0",
            "handler_name=Rewind mix",
            "-metadata:s:a:1",
            "handler_name=Game",
            "-metadata:s:a:2",
            "handler_name=Discord",
        ]);
    }
    let result = cmd
        .args([
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-pix_fmt",
            "yuv420p",
        ])
        .arg(&path)
        .output()
        .unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    media::probe("ffprobe", &path).unwrap()
}
fn edit(clip: Clip, start: f64, end: f64, audio: Vec<u32>) -> Export {
    Export {
        clip,
        output: Path::new("").to_owned(),
        start,
        end,
        quality: Quality::Bitrate,
        bitrate_mbps: 0.5,
        target_mb: 1.,
        max_height: None,
        encoding: EncodingOptions::default(),
        audio_tracks: Some(audio),
    }
}
fn sound(path: &Path, time: f64) -> (f64, f64, f64) {
    let output = media::command("ffmpeg")
        .args(["-v", "error", "-ss", &time.to_string(), "-i"])
        .arg(path)
        .args([
            "-t", "0.2", "-map", "0:a:0", "-ac", "1", "-ar", "48000", "-f", "f32le", "pipe:1",
        ])
        .output()
        .unwrap();
    assert!(output.status.success());
    let samples: Vec<_> = output
        .stdout
        .chunks_exact(4)
        .map(|v| f32::from_le_bytes(v.try_into().unwrap()) as f64)
        .collect();
    let power = |hz: f64| {
        let (mut a, mut b) = (0., 0.);
        for (i, s) in samples.iter().enumerate() {
            let p = std::f64::consts::TAU * hz * i as f64 / 48000.;
            a += s * p.sin();
            b += s * p.cos();
        }
        (a * a + b * b).sqrt() / samples.len() as f64
    };
    (
        power(440.),
        power(880.),
        (samples.iter().map(|s| s * s).sum::<f64>() / samples.len() as f64).sqrt(),
    )
}
#[test]
fn montage_keeps_order_exact_cuts_silence_and_per_application_audio_without_modifying_sources() {
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-movie-test").unwrap();
    let red = source(&work, "red", true);
    let blue = source(&work, "blue", false);
    let before = [
        std::fs::read(&red.path).unwrap(),
        std::fs::read(&blue.path).unwrap(),
    ];
    let mut compact = edit(red.clone(), 1.5, 2., vec![2]);
    compact.max_height = Some(90);
    compact.encoding.fps = Some(15);
    let parts = vec![
        Part {
            gap_before: 0.0,
            edit: edit(red.clone(), 0.5, 1.5, vec![1]),
            preprocess: false,
        },
        Part {
            gap_before: 0.0,
            edit: edit(blue.clone(), 0.25, 1.25, vec![]),
            preprocess: false,
        },
        Part {
            gap_before: 0.0,
            edit: compact,
            preprocess: true,
        },
    ];
    let mut output = edit(blue.clone(), 0., 2.5, vec![0]);
    output.output = work.0.join("movie.mp4");
    output.encoding.fps = Some(30);
    movie::render(
        "ffmpeg",
        "ffprobe",
        &parts,
        &output,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    let clip = media::probe("ffprobe", &output.output).unwrap();
    assert_eq!((clip.width, clip.height), (320, 180));
    assert!((clip.duration - 2.5).abs() < 0.01);
    assert_eq!(clip.fps, 30.);
    for (time, red) in [(0.4, true), (1.4, false), (2.2, true)] {
        let frame = media::command("ffmpeg")
            .args(["-v", "error", "-ss", &time.to_string(), "-i"])
            .arg(&output.output)
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
        assert!(frame.status.success());
        assert!(if red {
            frame.stdout[0] > 150 && frame.stdout[2] < 30
        } else {
            frame.stdout[2] > 150 && frame.stdout[0] < 30
        });
    }
    let (game, discord, _) = sound(&output.output, 0.25);
    assert!(game > 0.02 && discord < 0.001, "{game} {discord}");
    assert!(sound(&output.output, 1.25).2 < 0.0001);
    let (game, discord, _) = sound(&output.output, 2.2);
    assert!(discord > 0.02 && game < 0.001, "{game} {discord}");
    assert_eq!(std::fs::read(&red.path).unwrap(), before[0]);
    assert_eq!(std::fs::read(&blue.path).unwrap(), before[1]);
    output.output = work.0.join("target.mp4");
    output.quality = Quality::Target;
    output.target_mb = 0.3;
    movie::render(
        "ffmpeg",
        "ffprobe",
        &parts,
        &output,
        Arc::new(AtomicBool::new(false)),
    )
    .unwrap();
    assert!(media::probe("ffprobe", &output.output).unwrap().has_audio);
    output.output = work.0.join("cancel.mp4");
    assert!(
        movie::render(
            "ffmpeg",
            "ffprobe",
            &parts,
            &output,
            Arc::new(AtomicBool::new(true))
        )
        .is_err()
    );
    assert!(!output.output.exists());
    assert!(
        !std::fs::read_dir(&work.0)
            .unwrap()
            .any(|e| e.unwrap().file_type().unwrap().is_dir())
    );
    output.output = red.path.clone();
    assert!(
        movie::render(
            "ffmpeg",
            "ffprobe",
            &parts,
            &output,
            Arc::new(AtomicBool::new(false))
        )
        .is_err()
    );
}
#[test]
fn movie_supports_the_advertised_output_formats() {
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-movie-formats").unwrap();
    let clip = source(&work, "source", true);
    let parts = vec![Part {
        gap_before: 0.0,
        edit: edit(clip.clone(), 0.25, 0.75, vec![1]),
        preprocess: false,
    }];
    for (format, codec) in [
        (OutputFormat::Mkv, VideoCodec::Hevc),
        (OutputFormat::Webm, VideoCodec::Vp9),
        (OutputFormat::Gif, VideoCodec::H264),
    ] {
        let mut output = edit(clip.clone(), 0., 0.5, vec![0]);
        output.output = work.0.join(format!("movie.{}", format.extension()));
        output.encoding = EncodingOptions {
            format,
            codec,
            fps: Some(15),
            hardware: false,
        };
        movie::render(
            "ffmpeg",
            "ffprobe",
            &parts,
            &output,
            Arc::new(AtomicBool::new(false)),
        )
        .unwrap();
        let result = media::probe("ffprobe", &output.output).unwrap();
        assert!(result.duration >= 0.45);
        assert_eq!(result.has_audio, format != OutputFormat::Gif);
    }
}
