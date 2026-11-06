//! The recorder's single-video MPEG-TS pipe: encoded frames grouped by keyframe.
//! No capture data is written to disk. Snapshots share immutable frame storage.
use anyhow::{Result, bail};
use std::{
    collections::VecDeque,
    io::Write,
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

pub const TS_PACKET_SIZE: usize = 188;
pub const REPLAY_BYTE_LIMIT: usize = 256 * 1024 * 1024;
const VIDEO_PID: u16 = 256;
const PMT_PID: u16 = 4096;
const CLOCK: u64 = 90_000;
const PTS_MASK: u64 = (1 << 33) - 1;
const MAX_FRAME_BYTES: usize = 8 * 1024 * 1024;

struct PendingFrame {
    pts: u64,
    keyframe: bool,
    tables: [Arc<[u8]>; 2],
    data: Vec<u8>,
    wall_end: Option<Instant>,
}

struct Gop {
    pts: u64,
    tables: [Arc<[u8]>; 2],
    frames: Vec<Arc<[u8]>>,
    bytes: usize,
}

#[derive(Clone)]
pub struct ReplaySnapshot {
    tables: [Arc<[u8]>; 2],
    frames: Vec<Arc<[u8]>>,
    pub duration: f64,
    pub bytes: usize,
    pub source: Option<String>,
    pub trim_start: f64,
    pub fps: u32,
    pub end_time: Instant,
}

impl ReplaySnapshot {
    pub fn write_to(&self, writer: &mut impl Write) -> std::io::Result<()> {
        for table in &self.tables {
            writer.write_all(table)?;
        }
        for frame in &self.frames {
            writer.write_all(frame)?;
        }
        Ok(())
    }
}

pub struct ReplayBuffer {
    gops: VecDeque<Gop>,
    pending: Option<PendingFrame>,
    tables: [Arc<[u8]>; 2],
    byte_limit: usize,
    ticks_limit: u64,
    frame_ticks: u64,
    bytes: usize,
    last_pts: u64,
    previous_raw_pts: Option<u64>,
    clock: u64,
    timestamp_offset: u64,
    continuity: [u8; 8192],
    stream_frames: u64,
    stream_has_keyframe: bool,
    last_arrival: Option<Instant>,
    wallclock_timestamps: bool,
}

impl ReplayBuffer {
    pub fn new(seconds: u32, fps: u32, byte_limit: usize) -> Self {
        Self {
            gops: VecDeque::new(),
            pending: None,
            tables: [Arc::from([]), Arc::from([])],
            byte_limit,
            ticks_limit: seconds as u64 * CLOCK,
            frame_ticks: CLOCK / fps.max(1) as u64,
            bytes: 0,
            last_pts: 0,
            previous_raw_pts: None,
            clock: 0,
            timestamp_offset: 0,
            continuity: [0; 8192],
            stream_frames: 0,
            stream_has_keyframe: false,
            last_arrival: None,
            wallclock_timestamps: false,
        }
    }

    /// Only complete access units enter the replay; the current frame stays pending
    /// until the following video PES begins. Saving therefore never cuts a frame.
    pub fn push_packet(&mut self, packet: &[u8; TS_PACKET_SIZE]) -> Result<()> {
        if packet[0] != 0x47 || packet[1] & 0x80 != 0 {
            bail!("Повреждён поток записи MPEG-TS");
        }
        let pid = ((packet[1] as u16 & 0x1f) << 8) | packet[2] as u16;
        if pid == VIDEO_PID && packet[1] & 0x40 != 0 {
            let (pts, keyframe) = frame_header(packet)?;
            self.commit_frame();
            let wall_end = self.presentation_end(pts);
            let pts = self.unwrap_pts(pts, wall_end);
            if keyframe && self.tables.iter().all(|t| !t.is_empty()) {
                self.stream_has_keyframe = true;
            }
            if self.stream_has_keyframe && self.tables.iter().all(|t| !t.is_empty()) {
                self.pending = Some(PendingFrame {
                    pts,
                    keyframe,
                    tables: self.tables.clone(),
                    data: Vec::new(),
                    wall_end,
                });
            }
        }
        // Normalize timestamps and counters as packets enter the shared ring.
        // A restarted FFmpeg starts PTS/PCR/CC again, while the replay continues.
        let mut packet = *packet;
        normalize_timestamps(&mut packet, self.timestamp_offset);
        let counter = &mut self.continuity[pid as usize];
        if packet[3] & 0x10 != 0 {
            *counter = (*counter + 1) & 15;
        }
        packet[3] = (packet[3] & 0xf0) | *counter;
        if pid == 0 || pid == PMT_PID {
            self.tables[usize::from(pid == PMT_PID)] = Arc::from(packet.as_slice());
        }
        if let Some(pending) = &mut self.pending {
            pending.data.extend_from_slice(&packet);
            if pending.data.len() > MAX_FRAME_BYTES.min(self.byte_limit) {
                self.pending = None;
                self.clear_gops();
            }
        }
        self.enforce_memory_limit();
        Ok(())
    }

    pub fn enable_wallclock_timestamps(&mut self) {
        self.wallclock_timestamps = true;
    }

    fn presentation_end(&self, raw: u64) -> Option<Instant> {
        if !self.wallclock_timestamps {
            return None;
        }
        let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
        let clock =
            (now.as_secs() * CLOCK + now.subsec_nanos() as u64 * CLOCK / 1_000_000_000) & PTS_MASK;
        let age = clock.wrapping_sub(raw) & PTS_MASK;
        if age > 30 * CLOCK {
            return None;
        }
        Some(
            Instant::now() - Duration::from_secs_f64(age as f64 / CLOCK as f64)
                + Duration::from_secs_f64(self.frame_ticks as f64 / CLOCK as f64),
        )
    }

    fn unwrap_pts(&mut self, raw: u64, wall_end: Option<Instant>) -> u64 {
        if let Some(previous) = self.previous_raw_pts {
            let delta = raw.wrapping_sub(previous) & PTS_MASK;
            // bf=0 guarantees monotonic presentation. Handle the 33-bit wrap;
            // ignore a backwards timestamp instead of jumping forward 26 hours.
            if delta < 1 << 32 {
                self.clock = self.clock.saturating_add(delta);
            }
        } else {
            if !self.gops.is_empty() {
                let first_start = wall_end
                    .map(|end| {
                        end - Duration::from_secs_f64(self.frame_ticks as f64 / CLOCK as f64)
                    })
                    .unwrap_or_else(Instant::now);
                let gap = self.last_arrival.map_or(self.frame_ticks, |last| {
                    (first_start.saturating_duration_since(last).as_secs_f64() * CLOCK as f64)
                        as u64
                        + self.frame_ticks
                });
                self.clock = self.last_pts.saturating_add(gap.max(self.frame_ticks));
            }
            self.timestamp_offset = self.clock.wrapping_sub(raw) & PTS_MASK;
        }
        self.previous_raw_pts = Some(raw);
        self.clock
    }

    fn commit_frame(&mut self) {
        let Some(frame) = self.pending.take() else {
            return;
        };
        if frame.keyframe {
            self.gops.push_back(Gop {
                pts: frame.pts,
                tables: frame.tables,
                frames: vec![],
                bytes: 0,
            });
        }
        if let Some(gop) = self.gops.back_mut() {
            let bytes = frame.data.len();
            gop.bytes += bytes;
            gop.frames.push(Arc::from(frame.data));
            self.bytes += bytes;
            self.last_pts = frame.pts;
            self.last_arrival = Some(frame.wall_end.unwrap_or_else(Instant::now));
            self.stream_frames += 1;
        }
        // Retain the keyframe immediately preceding the requested time window.
        let cutoff = self.last_pts.saturating_sub(self.ticks_limit);
        while self.gops.len() > 1 && self.gops[1].pts <= cutoff {
            self.pop_front();
        }
        self.enforce_memory_limit();
    }

    fn pop_front(&mut self) {
        if let Some(gop) = self.gops.pop_front() {
            self.bytes -= gop.bytes;
        }
    }

    fn clear_gops(&mut self) {
        self.gops.clear();
        self.bytes = 0;
        self.stream_has_keyframe = false;
    }

    /// Discard only the unfinished access unit. Encoded history and snapshots
    /// survive capture loss; a new stream must start with its own tables and IDR.
    pub fn begin_stream(&mut self) {
        self.pending = None;
        self.tables = [Arc::from([]), Arc::from([])];
        self.previous_raw_pts = None;
        self.stream_frames = 0;
        self.stream_has_keyframe = false;
    }

    pub fn stream_duration(&self) -> f64 {
        (self.stream_frames * self.frame_ticks) as f64 / CLOCK as f64
    }

    fn enforce_memory_limit(&mut self) {
        let pending = self.pending.as_ref().map_or(0, |p| p.data.len());
        while self.bytes + pending > self.byte_limit && self.gops.len() > 1 {
            self.pop_front();
        }
        // An oversized GOP cannot be decoded after removing its keyframe.
        // Drop it as a whole and wait for the next keyframe, keeping RAM bounded.
        if self.bytes + pending > self.byte_limit {
            self.clear_gops();
            self.pending = None;
        }
    }

    pub fn duration(&self) -> f64 {
        self.gops.front().map_or(0.0, |gop| {
            (self.last_pts.saturating_sub(gop.pts) + self.frame_ticks) as f64 / CLOCK as f64
        })
    }
    pub fn live_duration(&self, now: Instant) -> f64 {
        if self.gops.is_empty() {
            return 0.0;
        }
        self.duration()
            + self.last_arrival.map_or(0.0, |last| {
                now.saturating_duration_since(last).as_secs_f64()
            })
    }

    /// Resize the time window without touching the capture process or snapshots.
    pub fn set_seconds(&mut self, seconds: u32) {
        self.ticks_limit = seconds as u64 * CLOCK;
        let cutoff = self.last_pts.saturating_sub(self.ticks_limit);
        while self.gops.len() > 1 && self.gops[1].pts <= cutoff {
            self.pop_front();
        }
    }

    pub fn buffered_bytes(&self) -> usize {
        self.bytes + self.pending.as_ref().map_or(0, |p| p.data.len())
    }

    pub fn snapshot(&self) -> Result<ReplaySnapshot> {
        self.make_snapshot(0, self.last_arrival.unwrap_or_else(Instant::now))
    }

    pub fn live_snapshot(&self, now: Instant) -> Result<ReplaySnapshot> {
        let stale = self.last_arrival.map_or(0, |last| {
            (now.saturating_duration_since(last).as_secs_f64() * CLOCK as f64) as u64
        });
        self.make_snapshot(stale, now)
    }

    fn make_snapshot(&self, extra_ticks: u64, end_time: Instant) -> Result<ReplaySnapshot> {
        let first = self
            .gops
            .front()
            .ok_or_else(|| anyhow::anyhow!("Буфер ещё пуст"))?;
        let end = self
            .last_pts
            .saturating_add(self.frame_ticks)
            .saturating_add(extra_ticks);
        let available = end.saturating_sub(first.pts);
        let duration = available.min(self.ticks_limit);
        Ok(ReplaySnapshot {
            tables: first.tables.clone(),
            frames: self
                .gops
                .iter()
                .flat_map(|g| g.frames.iter().cloned())
                .collect(),
            duration: duration as f64 / CLOCK as f64,
            bytes: self.bytes + 2 * TS_PACKET_SIZE,
            source: None,
            trim_start: (available - duration) as f64 / CLOCK as f64,
            fps: (CLOCK / self.frame_ticks) as u32,
            end_time,
        })
    }

    /// For a naturally completed finite stream only, not an interrupted capture.
    pub fn finish_stream(&mut self) {
        self.commit_frame();
    }
}

fn read_timestamp(p: &[u8]) -> u64 {
    ((p[0] as u64 & 0x0e) << 29)
        | ((p[1] as u64) << 22)
        | ((p[2] as u64 & 0xfe) << 14)
        | ((p[3] as u64) << 7)
        | ((p[4] as u64) >> 1)
}

fn shift_timestamp(p: &mut [u8], offset: u64) {
    let pts = read_timestamp(p).wrapping_add(offset) & PTS_MASK;
    p[0] = (p[0] & 0xf1) | ((pts >> 29) as u8 & 0x0e);
    p[1] = (pts >> 22) as u8;
    p[2] = ((pts >> 14) as u8 & 0xfe) | (p[2] & 1);
    p[3] = (pts >> 7) as u8;
    p[4] = ((pts << 1) as u8) | (p[4] & 1);
}

fn normalize_timestamps(packet: &mut [u8; TS_PACKET_SIZE], offset: u64) {
    if packet[3] & 0x20 != 0 && packet[4] >= 7 && packet[5] & 0x10 != 0 {
        let p = &mut packet[6..12];
        let base = ((p[0] as u64) << 25)
            | ((p[1] as u64) << 17)
            | ((p[2] as u64) << 9)
            | ((p[3] as u64) << 1)
            | ((p[4] as u64) >> 7);
        let base = base.wrapping_add(offset) & PTS_MASK;
        p[0] = (base >> 25) as u8;
        p[1] = (base >> 17) as u8;
        p[2] = (base >> 9) as u8;
        p[3] = (base >> 1) as u8;
        p[4] = ((base << 7) as u8) | (p[4] & 0x7f);
    }
    let pid = ((packet[1] as u16 & 0x1f) << 8) | packet[2] as u16;
    if pid != VIDEO_PID || packet[1] & 0x40 == 0 || packet[3] & 0x10 == 0 {
        return;
    }
    let start = 4 + if packet[3] & 0x20 != 0 {
        packet[4] as usize + 1
    } else {
        0
    };
    if start + 14 <= TS_PACKET_SIZE && packet[start + 7] & 0x80 != 0 {
        shift_timestamp(&mut packet[start + 9..start + 14], offset);
        if packet[start + 7] & 0x40 != 0 && start + 19 <= TS_PACKET_SIZE {
            shift_timestamp(&mut packet[start + 14..start + 19], offset);
        }
    }
}

fn frame_header(packet: &[u8; TS_PACKET_SIZE]) -> Result<(u64, bool)> {
    let has_adaptation = packet[3] & 0x20 != 0;
    let adaptation_len = if has_adaptation {
        packet[4] as usize + 1
    } else {
        0
    };
    let start = 4 + adaptation_len;
    if packet[3] & 0x10 == 0 || start + 14 > TS_PACKET_SIZE {
        bail!("Неверный заголовок кадра записи");
    }
    let payload = &packet[start..];
    if payload[..3] != [0, 0, 1] || payload[3] & 0xf0 != 0xe0 || payload[7] & 0x80 == 0 {
        bail!("В потоке записи отсутствует время видеокадра");
    }
    let pts = read_timestamp(&payload[9..14]);
    let keyframe = has_adaptation && packet[4] > 0 && packet[5] & 0x40 != 0;
    Ok((pts, keyframe))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn packet(pts: u64, keyframe: bool) -> [u8; TS_PACKET_SIZE] {
        let mut packet = [0xff; TS_PACKET_SIZE];
        packet[..6].copy_from_slice(&[0x47, 0x41, 0, 0x30, 1, if keyframe { 0x40 } else { 0 }]);
        packet[6..15].copy_from_slice(&[0, 0, 1, 0xe0, 0, 0, 0x80, 0x80, 5]);
        packet[15..20].copy_from_slice(&[
            0x21 | (((pts >> 30) as u8 & 7) << 1),
            (pts >> 22) as u8,
            ((pts >> 14) as u8 & 0xfe) | 1,
            (pts >> 7) as u8,
            ((pts << 1) as u8) | 1,
        ]);
        packet
    }
    fn buffer(seconds: u32, limit: usize) -> ReplayBuffer {
        let mut buffer = ReplayBuffer::new(seconds, 30, limit);
        for pid in [0_u16, PMT_PID] {
            let mut p = [0; TS_PACKET_SIZE];
            p[0] = 0x47;
            p[1] = (pid >> 8) as u8;
            p[2] = pid as u8;
            buffer.push_packet(&p).unwrap();
        }
        buffer
    }
    #[test]
    fn evicts_whole_gops_and_keeps_snapshots_alive() {
        let mut ring = buffer(3, 1_000_000);
        for i in 0..=120 {
            ring.push_packet(&packet(i * 3000, i % 60 == 0)).unwrap();
        }
        let snapshot = ring.snapshot().unwrap();
        assert!((snapshot.duration - 3.0).abs() < 0.01);
        assert!((snapshot.trim_start - 1.0).abs() < 0.01);
        let mut original = Vec::new();
        snapshot.write_to(&mut original).unwrap();
        for i in 121..=300 {
            ring.push_packet(&packet(i * 3000, i % 60 == 0)).unwrap();
        }
        assert!(ring.duration() >= 3.0 && ring.duration() <= 5.0);
        let mut preserved = Vec::new();
        snapshot.write_to(&mut preserved).unwrap();
        assert_eq!(preserved, original);
        assert_eq!(preserved.len(), snapshot.bytes);
        assert_ne!(
            ring.snapshot().unwrap().frames[0].as_ref(),
            snapshot.frames[0].as_ref()
        );
    }
    #[test]
    fn caps_memory_even_if_no_more_keyframes_arrive() {
        let mut ring = buffer(120, TS_PACKET_SIZE * 90);
        for i in 0..1000 {
            ring.push_packet(&packet(i * 3000, i == 0)).unwrap();
            assert!(ring.buffered_bytes() <= TS_PACKET_SIZE * 90);
        }
        assert!(ring.snapshot().is_err());
        ring.push_packet(&packet(1000 * 3000, true)).unwrap();
        ring.push_packet(&packet(1001 * 3000, false)).unwrap();
        assert!(ring.snapshot().is_ok());
    }
    #[test]
    fn resizing_a_live_buffer_preserves_capture_and_existing_snapshots() {
        let mut ring = buffer(120, 1_000_000);
        for i in 0..=300 {
            ring.push_packet(&packet(i * 3000, i % 30 == 0)).unwrap();
        }
        let snapshot = ring.snapshot().unwrap();
        let mut original = Vec::new();
        snapshot.write_to(&mut original).unwrap();
        ring.set_seconds(3);
        assert!(ring.duration() >= 3.0 && ring.duration() <= 4.0);
        ring.set_seconds(120);
        for i in 301..=600 {
            ring.push_packet(&packet(i * 3000, i % 30 == 0)).unwrap();
        }
        assert!(ring.duration() > 10.0);
        let mut preserved = Vec::new();
        snapshot.write_to(&mut preserved).unwrap();
        assert_eq!(preserved, original);
    }
    #[test]
    fn timestamps_wrap_and_partial_frames_are_excluded() {
        let mut ring = buffer(30, 1_000_000);
        ring.push_packet(&packet(PTS_MASK - 2999, true)).unwrap();
        assert!(ring.snapshot().is_err());
        ring.push_packet(&packet(0, false)).unwrap();
        assert!((ring.duration() - 1.0 / 30.0).abs() < 0.001);
        ring.push_packet(&packet(3000, false)).unwrap();
        assert!((ring.duration() - 2.0 / 30.0).abs() < 0.001);
        ring.finish_stream();
        assert!((ring.duration() - 0.1).abs() < 0.001);
        assert!(ring.push_packet(&[0; TS_PACKET_SIZE]).is_err());
    }

    #[test]
    fn restarting_a_stream_retains_history_and_rebases_timestamps() {
        let mut ring = buffer(30, 1_000_000);
        for i in 0..=60 {
            ring.push_packet(&packet(90_000 + i * 3000, i % 30 == 0))
                .unwrap();
        }
        let before = ring.snapshot().unwrap();
        let mut original = Vec::new();
        before.write_to(&mut original).unwrap();
        ring.begin_stream();
        assert_eq!(ring.stream_duration(), 0.0);
        assert_eq!(ring.snapshot().unwrap().bytes, before.bytes);
        for pid in [0_u16, PMT_PID] {
            let mut table = [0; TS_PACKET_SIZE];
            table[0] = 0x47;
            table[1] = (pid >> 8) as u8;
            table[2] = pid as u8;
            ring.push_packet(&table).unwrap();
        }
        // P-frames from a fresh stream cannot attach to an old GOP.
        ring.push_packet(&packet(0, false)).unwrap();
        ring.push_packet(&packet(3000, false)).unwrap();
        assert_eq!(ring.snapshot().unwrap().bytes, before.bytes);
        for i in 2..=62 {
            ring.push_packet(&packet(i * 3000, i == 2 || i == 32 || i == 62))
                .unwrap();
        }
        let after = ring.snapshot().unwrap();
        assert!(after.duration >= 3.9 && after.duration < 4.2);
        let mut bytes = Vec::new();
        after.write_to(&mut bytes).unwrap();
        let pts: Vec<_> = bytes
            .chunks_exact(TS_PACKET_SIZE)
            .filter(|packet| packet[1] & 0x40 != 0)
            .map(|packet| frame_header(packet.try_into().unwrap()).unwrap().0)
            .collect();
        assert!(pts.windows(2).all(|pair| pair[1] > pair[0]));
        let mut unchanged = Vec::new();
        before.write_to(&mut unchanged).unwrap();
        assert_eq!(unchanged, original);
    }

    #[test]
    fn timestamp_rebasing_preserves_pes_flags_and_pcr_extension() {
        let mut packet = packet(PTS_MASK - 2999, true);
        normalize_timestamps(&mut packet, 3000);
        assert_eq!(frame_header(&packet).unwrap(), (0, true));
        // PCR base=90000, extension=17; no payload in this transport packet.
        packet = [0; TS_PACKET_SIZE];
        packet[..6].copy_from_slice(&[0x47, 1, 0, 0x20, 7, 0x10]);
        let base = 90_000_u64;
        packet[6..12].copy_from_slice(&[
            (base >> 25) as u8,
            (base >> 17) as u8,
            (base >> 9) as u8,
            (base >> 1) as u8,
            ((base << 7) as u8) | 0x7e,
            17,
        ]);
        normalize_timestamps(&mut packet, 90_000);
        let p = &packet[6..12];
        let adjusted = ((p[0] as u64) << 25)
            | ((p[1] as u64) << 17)
            | ((p[2] as u64) << 9)
            | ((p[3] as u64) << 1)
            | ((p[4] as u64) >> 7);
        assert_eq!(adjusted, 180_000);
        assert_eq!(p[4] & 0x7f, 0x7e);
        assert_eq!(p[5], 17);
    }

    #[test]
    fn exact_saved_window_keeps_decoder_preroll_for_a_long_leading_gop() {
        let mut ring = buffer(45, 1_000_000);
        // The reported clip: first IDR at 0, next at ~44 s, end at ~67 s.
        for (seconds, keyframe) in [
            (0, true),
            (20, false),
            (43, false),
            (44, true),
            (46, true),
            (60, true),
            (66, false),
            (67, false),
        ] {
            ring.push_packet(&packet(seconds * CLOCK, keyframe))
                .unwrap();
        }
        ring.finish_stream();
        assert!(
            ring.duration() > 66.0,
            "Internal decoder preroll is retained"
        );
        let snapshot = ring.snapshot().unwrap();
        assert_eq!(snapshot.duration, 45.0);
        assert!((snapshot.trim_start - (22.0 + 1.0 / 30.0)).abs() < 0.001);
        let mut bytes = Vec::new();
        snapshot.write_to(&mut bytes).unwrap();
        let first = bytes
            .chunks_exact(TS_PACKET_SIZE)
            .find(|p| p[1] & 0x40 != 0)
            .unwrap();
        assert_eq!(frame_header(first.try_into().unwrap()).unwrap(), (0, true));
        assert_eq!(bytes.len(), snapshot.bytes);
    }

    #[test]
    fn saved_window_includes_last_frame_duration_and_live_resize() {
        let mut ring = buffer(60, 1_000_000);
        for i in 0..=1800 {
            ring.push_packet(&packet(i * 3000, i % 60 == 0)).unwrap();
        }
        ring.finish_stream();
        let preserved = ring.snapshot().unwrap();
        ring.set_seconds(45);
        assert!(ring.snapshot().unwrap().duration <= 45.0);
        ring.set_seconds(30);
        assert!(ring.snapshot().unwrap().duration <= 30.0);
        assert!(
            preserved.duration > 45.0,
            "A saved snapshot keeps its original window"
        );
    }
}
