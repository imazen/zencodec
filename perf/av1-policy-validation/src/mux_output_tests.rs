use super::*;
use crate::fixtures::*;
use enough::Unstoppable;
use std::{fs::OpenOptions, path::PathBuf};
use zencodec::time::TimeBase;
use zencodec_media::{
    container::{MuxFinish, Patch, PatchError},
    frame::Subsampling,
    session::CodecConfig,
};

fn root() -> PathBuf {
    let root = PathBuf::from(
        std::env::var_os("AV1_POLICY_ARTIFACTS").expect("persistent artifact directory required"),
    );
    std::fs::create_dir_all(&root).unwrap();
    root
}

fn track(config: &CodecConfig) -> TrackInfo {
    let mut track = TrackInfo::video(
        0,
        config.codec.clone(),
        config.time_base,
        config.video.unwrap(),
    )
    .with_config_epoch(config.epoch);
    if let Some(private) = &config.private {
        track = track.with_codec_private(private.clone());
    }
    track.build().unwrap()
}

fn inspect_timing(name: &str, extension: &str, expected: &[Packet]) -> usize {
    let dir = root().join(name);
    let result = std::process::Command::new("timeout")
        .args([
            "--kill-after=5s",
            "30s",
            "ffprobe",
            "-v",
            "error",
            // Inspect the container's timing, without parser duration rewriting
            // or inferred timestamps. FFmpeg 6's AV1 parser otherwise clears
            // WebM BlockDuration on packets without COMPLETE_FRAMES.
            "-fflags",
            "+noparse+nofillin",
            "-select_streams",
            "v:0",
            "-show_entries",
            "stream=time_base",
            "-show_entries",
            "packet=pts,dts,duration",
            "-of",
            "compact=p=1:nk=0",
        ])
        .arg(dir.join(format!("approved.{extension}")))
        .output()
        .unwrap();
    std::fs::write(dir.join("ffprobe.stdout"), &result.stdout).unwrap();
    std::fs::write(dir.join("ffprobe.stderr"), &result.stderr).unwrap();
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    let text = String::from_utf8(result.stdout).unwrap();
    let clock = text
        .lines()
        .find_map(|line| line.strip_prefix("stream|time_base="))
        .unwrap();
    let (n, d) = clock.split_once('/').unwrap();
    let (n, d): (i128, i128) = (n.parse().unwrap(), d.parse().unwrap());
    let packets: Vec<_> = text
        .lines()
        .filter(|line| line.starts_with("packet|"))
        .collect();
    assert_eq!(packets.len(), expected.len());
    let mut durations = 0;
    for (line, expected) in packets.iter().zip(expected) {
        let value = |key: &str| {
            line.split('|')
                .find_map(|part| part.strip_prefix(key))
                .unwrap()
        };
        let pts: i128 = value("pts=").parse().unwrap();
        let original = expected.pts().unwrap();
        assert_eq!(
            pts * n * i128::from(original.base().denominator()),
            i128::from(original.ticks()) * i128::from(original.base().numerator()) * d
        );
        if let Ok(duration) = value("duration=").parse::<i128>() {
            assert_eq!(
                duration * n * i128::from(original.base().denominator()),
                i128::from(expected.duration_ticks().unwrap())
                    * i128::from(original.base().numerator())
                    * d
            );
            durations += 1;
        }
    }
    durations
}

struct Short<W> {
    writer: W,
    interrupted: bool,
}
impl<W: Write> Write for Short<W> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if !self.interrupted {
            self.interrupted = true;
            return Err(io::ErrorKind::Interrupted.into());
        }
        self.writer.write(&bytes[..bytes.len().min(3)])
    }
    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}
impl<W: Seek> Seek for Short<W> {
    fn seek(&mut self, from: SeekFrom) -> io::Result<u64> {
        self.writer.seek(from)
    }
}

fn run<M: Muxer>(make: impl Fn() -> M, extension: &str, forward: bool, bits: u8) -> String {
    let root = root();
    let name = format!("mux-{extension}-{forward}-{bits}");
    let settings = settings(bits, Subsampling::Yuv420, false);
    let frames: Vec<_> = (0..6).map(|i| frame(&settings, i)).collect();
    // encode drains and drops the native encoder before the first writer call.
    let (config, packets) = encode(settings.clone(), &frames);
    let expected_timing = packets.clone();
    let track = track(&config);
    let (bytes, stats) = if forward {
        let mut bytes = Vec::new();
        let destination = Forward(Short {
            writer: &mut bytes,
            interrupted: false,
        });
        let mut output = Output::new(make(), destination, &[track], &Unstoppable).unwrap();
        for packet in packets {
            output.send(packet, &Unstoppable).unwrap();
        }
        let stats = output.finish(&Unstoppable).unwrap();
        assert_eq!(stats.patches, 0);
        (bytes, stats)
    } else {
        let path = root.join(format!("{name}.embedded"));
        let mut file = OpenOptions::new()
            .write(true)
            .read(true)
            .create_new(true)
            .open(&path)
            .unwrap();
        let prefix = b"preserve preceding content";
        file.write_all(prefix).unwrap();
        let destination = SeekBack::new(Short {
            writer: &mut file,
            interrupted: false,
        })
        .unwrap();
        let mut output = Output::new(make(), destination, &[track], &Unstoppable).unwrap();
        for packet in packets {
            output.send(packet, &Unstoppable).unwrap();
        }
        let stats = output.finish(&Unstoppable).unwrap();
        assert_eq!(
            file.stream_position().unwrap(),
            prefix.len() as u64 + stats.emitted
        );
        drop(file);
        let embedded = std::fs::read(path).unwrap();
        assert_eq!(&embedded[..prefix.len()], prefix);
        (embedded[prefix.len()..].to_vec(), stats)
    };
    assert_eq!(stats.emitted as usize, bytes.len());
    if !forward {
        assert_eq!(stats.patches, usize::from(extension != "webm"));
    }
    oracle_container(&root, &name, extension, &settings, &bytes, &frames);
    let durations = inspect_timing(&name, extension, &expected_timing);
    assert_eq!(durations, if extension == "ivf" { 0 } else { frames.len() });
    format!(
        "{extension},{forward},{bits},{},{},{},{},{durations}\n",
        stats.emitted, stats.patches, stats.patch_bytes, stats.max_pending
    )
}

#[test]
fn mux_native_network_and_prefixed_disk_preserve_av1_samples() {
    let mut rows = String::from(
        "container,forward_only,bits,output_bytes,patches,patch_bytes,max_observed_pending_bytes,explicit_durations_preserved\n",
    );
    for bits in [8, 10] {
        for forward in [false, true] {
            rows.push_str(&run(
                || zenivf::IvfMuxer::new().with_count_patch(),
                "ivf",
                forward,
                bits,
            ));
            rows.push_str(&run(zenwebm::WebmMuxer::new, "webm", forward, bits));
            rows.push_str(&run(zenmp4::Mp4Muxer::new, "mp4", forward, bits));
        }
    }
    let path = root().join("mux-observations.csv");
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .unwrap();
    file.write_all(rows.as_bytes()).unwrap();
}

fn mismatched<M: Muxer>(make: impl Fn() -> M) {
    let settings = settings(8, Subsampling::Yuv420, false);
    let frames: Vec<_> = (0..2).map(|i| frame(&settings, i)).collect();
    let (config, packets) = encode(settings, &frames);
    for epoch in [false, true] {
        let mut bytes = Vec::new();
        let mut output =
            Output::new(make(), Forward(&mut bytes), &[track(&config)], &Unstoppable).unwrap();
        let before = output.stats.emitted;
        let bad = if epoch {
            packets[0].clone().with_config_epoch(config.epoch + 1)
        } else {
            packets[0].clone().with_pts(zencodec::time::Timestamp::new(
                0,
                TimeBase::hz(90_000).unwrap(),
            ))
        };
        let failure = output.send(bad, &Unstoppable).unwrap_err();
        assert_eq!(failure.emitted, before);
        assert!(matches!(failure.stage, Stage::Packet));
        let expected = if epoch {
            ProtocolViolation::EpochMismatch
        } else {
            ProtocolViolation::ClockMismatch
        };
        assert!(
            matches!(failure.cause.downcast_ref::<SessionError<ErasedError>>(), Some(SessionError::Protocol(actual)) if *actual == expected)
        );
        assert!(output.is_failed());
        assert!(output.send(packets[1].clone(), &Unstoppable).is_err());
        drop(output);
        assert_eq!(bytes.len() as u64, before);
    }
}

#[test]
fn mux_native_epoch_and_clock_changes_refuse_before_packet_effects() {
    mismatched(zenivf::IvfMuxer::new);
    mismatched(zenwebm::WebmMuxer::new);
    mismatched(zenmp4::Mp4Muxer::new);
}

struct Broken<'a> {
    bytes: &'a mut Vec<u8>,
    remaining: usize,
}
impl Write for Broken<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if self.remaining == 0 {
            return Err(io::ErrorKind::BrokenPipe.into());
        }
        let n = bytes.len().min(self.remaining);
        self.bytes.extend_from_slice(&bytes[..n]);
        self.remaining -= n;
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn partial_header<M: Muxer>(make: impl Fn() -> M) {
    let settings = settings(8, Subsampling::Yuv420, false);
    let (config, _) = encode(settings.clone(), &[frame(&settings, 0)]);
    let mut bytes = Vec::new();
    let error = match Output::new(
        make(),
        Forward(Broken {
            bytes: &mut bytes,
            remaining: 13,
        }),
        &[track(&config)],
        &Unstoppable,
    ) {
        Ok(_) => panic!("partial header must fail"),
        Err(e) => e,
    };
    assert!(matches!(error.stage, Stage::Drain));
    assert_eq!(error.emitted, 13);
    assert_eq!(
        zencodec::find_cause::<io::Error>(&error).unwrap().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(bytes.len(), 13);
}

#[test]
fn mux_native_partial_header_write_keeps_io_cause_and_exact_effect_count() {
    partial_header(zenivf::IvfMuxer::new);
    partial_header(zenwebm::WebmMuxer::new);
    partial_header(zenmp4::Mp4Muxer::new);
}

struct BadFinish<M>(M);
impl<M: Muxer> Muxer for BadFinish<M> {
    type Error = M::Error;
    fn begin(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<Self::Error>> {
        self.0.begin(tracks, hint)
    }
    fn write_packet(
        &mut self,
        packet: Packet,
        stop: &dyn Stop,
    ) -> Result<(), SessionError<Self::Error>> {
        self.0.write_packet(packet, stop)
    }
    fn pending_output(&self) -> &[u8] {
        self.0.pending_output()
    }
    fn consume_output(&mut self, n: usize) {
        self.0.consume_output(n);
    }
    fn take_output(&mut self, into: &mut Vec<u8>) {
        self.0.take_output(into);
    }
    fn finish(self, stop: &dyn Stop) -> Result<MuxFinish, SessionError<Self::Error>> {
        let mut finish = self.0.finish(stop)?;
        finish.patches.push(Patch {
            offset: 0,
            bytes: b"BAD".to_vec(),
        });
        finish.patches.push(Patch {
            offset: u64::MAX,
            bytes: vec![1],
        });
        Ok(finish)
    }
}

#[test]
fn mux_native_invalid_final_patch_changes_neither_tail_nor_previous_bytes() {
    let settings = settings(8, Subsampling::Yuv420, false);
    let frames: Vec<_> = (0..6).map(|i| frame(&settings, i)).collect();
    let (config, packets) = encode(settings, &frames);
    let path = root().join("mux-invalid-patch.mp4");
    let mut file = OpenOptions::new()
        .write(true)
        .read(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    let mut output = Output::new(
        BadFinish(zenmp4::Mp4Muxer::new()),
        SeekBack::new(&mut file).unwrap(),
        &[track(&config)],
        &Unstoppable,
    )
    .unwrap();
    for packet in packets {
        output.send(packet, &Unstoppable).unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let failure = output.finish(&Unstoppable).unwrap_err();
    assert_eq!(
        zencodec::find_cause::<PatchError>(&failure),
        Some(&PatchError::OutOfRange)
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(file.stream_position().unwrap(), before.len() as u64);
}

fn shifted_origin<M: Muxer>(make: impl Fn() -> M, extension: &str, forward: bool) {
    let settings = settings(8, Subsampling::Yuv420, false);
    let frames: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
    let (config, packets) = encode(settings.clone(), &frames);
    let packets: Vec<_> = packets
        .into_iter()
        .map(|p| {
            let pts = p.pts().unwrap();
            // This fixture authors an explicit decode clock when the native
            // intra-only packets supply only their presentation clock.
            let dts = p.dts().unwrap_or(pts);
            p.with_pts(zencodec::time::Timestamp::new(pts.ticks() + 40, pts.base()))
                .with_dts(zencodec::time::Timestamp::new(dts.ticks() + 40, dts.base()))
        })
        .collect();
    let mut writer = std::io::Cursor::new(Vec::new());
    let result = if forward {
        let mut output = Output::new(
            make(),
            Forward(&mut writer),
            &[track(&config)],
            &Unstoppable,
        )
        .unwrap();
        for packet in &packets {
            output.send(packet.clone(), &Unstoppable).unwrap();
        }
        output.finish(&Unstoppable)
    } else {
        let mut output = Output::new(
            make(),
            SeekBack::new(&mut writer).unwrap(),
            &[track(&config)],
            &Unstoppable,
        )
        .unwrap();
        for packet in &packets {
            output.send(packet.clone(), &Unstoppable).unwrap();
        }
        output.finish(&Unstoppable)
    };
    let name = format!("mux-origin-{extension}-{forward}");
    if extension == "mp4" && !forward {
        let error = result.unwrap_err();
        assert!(matches!(error.stage, Stage::Finish));
        assert!(
            error.emitted > 0,
            "native begin did not certify this timestamp origin"
        );
        let native = zencodec::find_cause::<SessionError<zenmp4::Mp4Error>>(&error).unwrap();
        assert!(matches!(
            native,
            SessionError::Unsupported(UnsupportedMedia::Container)
        ));
        let path = root().join(format!("{name}.refusal"));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        writeln!(
            file,
            "{error}; native cause: {native}; already appended={}",
            writer.get_ref().len()
        )
        .unwrap();
    } else {
        result.unwrap();
        oracle_container(
            &root(),
            &name,
            extension,
            &settings,
            writer.get_ref(),
            &frames,
        );
        assert_eq!(
            inspect_timing(&name, extension, &packets),
            if extension == "ivf" { 0 } else { frames.len() }
        );
    }
}

#[test]
fn mux_native_shifted_origin_qualifies_actual_layout_and_preserves_late_refusal() {
    for forward in [false, true] {
        shifted_origin(zenivf::IvfMuxer::new, "ivf", forward);
        shifted_origin(zenwebm::WebmMuxer::new, "webm", forward);
        shifted_origin(zenmp4::Mp4Muxer::new, "mp4", forward);
    }
}

struct StopAfterWrite {
    armed: std::sync::atomic::AtomicBool,
    stopped: std::sync::atomic::AtomicBool,
    reason: enough::StopReason,
}
impl Stop for StopAfterWrite {
    fn check(&self) -> Result<(), enough::StopReason> {
        if self.stopped.load(std::sync::atomic::Ordering::SeqCst) {
            Err(self.reason)
        } else {
            Ok(())
        }
    }
}
struct TrippingWriter<'a> {
    bytes: &'a mut Vec<u8>,
    stop: &'a StopAfterWrite,
}
impl Write for TrippingWriter<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let n = bytes.len().min(7);
        self.bytes.extend_from_slice(&bytes[..n]);
        if self.stop.armed.load(std::sync::atomic::Ordering::SeqCst) {
            self.stop
                .stopped
                .store(true, std::sync::atomic::Ordering::SeqCst);
        }
        Ok(n)
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn mux_native_stop_during_packet_write_keeps_reason_and_cannot_resume() {
    let settings = settings(8, Subsampling::Yuv420, false);
    let (config, packets) = encode(settings.clone(), &[frame(&settings, 0)]);
    for reason in [enough::StopReason::Cancelled, enough::StopReason::TimedOut] {
        let stop = StopAfterWrite {
            armed: std::sync::atomic::AtomicBool::new(false),
            stopped: std::sync::atomic::AtomicBool::new(false),
            reason,
        };
        let mut bytes = Vec::new();
        let mut output = Output::new(
            zenivf::IvfMuxer::new(),
            Forward(TrippingWriter {
                bytes: &mut bytes,
                stop: &stop,
            }),
            &[track(&config)],
            &stop,
        )
        .unwrap();
        let header = output.stats.emitted;
        stop.armed.store(true, std::sync::atomic::Ordering::SeqCst);
        let error = output.send(packets[0].clone(), &stop).unwrap_err();
        assert_eq!(error.emitted, header + 7);
        assert!(
            matches!(zencodec::find_cause::<SessionError<ErasedError>>(&error), Some(SessionError::Cancelled(actual)) if *actual == reason)
        );
        assert!(output.is_failed());
        assert!(output.send(packets[0].clone(), &Unstoppable).is_err());
        drop(output);
        assert_eq!(bytes.len() as u64, error.emitted);
    }
}
