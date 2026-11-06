use rewind::media::{self, WorkDir};
use std::sync::atomic::AtomicBool;
#[test]
fn non_aac_preview_is_playable_and_cancellation_leaves_no_partial_cache() {
    let work = WorkDir::new(&std::env::temp_dir(), "rewind-preview-test").unwrap();
    let source = work.0.join("pcm.mkv");
    let encoded = media::command("ffmpeg")
        .args([
            "-v",
            "error",
            "-f",
            "lavfi",
            "-i",
            "color=s=160x90:r=30:d=2",
            "-f",
            "lavfi",
            "-i",
            "sine=frequency=440:sample_rate=48000:duration=2",
            "-map",
            "0:v",
            "-map",
            "1:a",
            "-c:v",
            "libx264",
            "-preset",
            "ultrafast",
            "-c:a",
            "pcm_s16le",
        ])
        .arg(&source)
        .output()
        .unwrap();
    assert!(encoded.status.success());
    let clip = media::probe("ffprobe", &source).unwrap();
    let previews =
        media::prepare_audio_tracks("ffmpeg", &clip, &work.0, &AtomicBool::new(false)).unwrap();
    assert_eq!(previews.len(), 1);
    let decoded = media::command("ffmpeg")
        .args(["-v", "error", "-xerror", "-i"])
        .arg(&previews[0].path)
        .args(["-f", "f32le", "pipe:1"])
        .output()
        .unwrap();
    assert!(decoded.status.success());
    assert!(decoded.stdout.len() > 48_000 * 4);
    let cancelled = WorkDir::new(&work.0, "cancelled").unwrap();
    assert!(
        media::prepare_audio_tracks("ffmpeg", &clip, &cancelled.0, &AtomicBool::new(true)).is_err()
    );
    assert_eq!(std::fs::read_dir(&cancelled.0).unwrap().count(), 0);
}
