use super::*;
use crate::fixtures::*;
use std::{fs::OpenOptions, path::PathBuf};
use zencodec::how_far::NoPulse;
use zencodec::time::TimeBase;
use zencodec_media::{
    container::{MuxFinish, Patch, PatchError},
    frame::Subsampling,
    session::CodecConfig,
};
use zenpixels::sample::SampleEncoding;

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

#[test]
fn mux_native_checked_legacy_support_refuses_before_header() {
    let mut bytes = Vec::new();
    let failure = match Output::new(
        zenivf::IvfMuxer::new(),
        Forward(&mut bytes),
        &[],
        Some(&NoPulse),
    ) {
        Ok(_) => panic!("unqualified implementation was admitted"),
        Err(failure) => failure,
    };
    assert!(matches!(failure.stage, Stage::Prepare));
    assert_eq!(failure.emitted, 0);
    assert!(bytes.is_empty());
    assert!(matches!(
        zencodec::find_cause::<SessionError<zenivf::IvfError>>(&failure),
        Some(SessionError::Unsupported(UnsupportedMedia::ExactTiming))
    ));
}

fn inspect_timing(name: &str, extension: &str, expected: &[Packet]) -> usize {
    let dir = root().join(name);
    let result = std::process::Command::new("timeout")
        .args(["--kill-after=5s", "30s"])
        .arg(std::env::var_os("AV1_TIMING_FFPROBE").unwrap_or_else(|| "ffprobe".into()))
        .args([
            "-v",
            "error",
            // Inspect the container's timing, without parser duration rewriting
            // or inferred timestamps. CI pins the timing inspector separately
            // from the system decoder; FFmpeg 6 omits fragmented MP4 durations.
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

// The native repo still pins the released media-trait revision. This thin
// integration shim connects its inherent exact-timing implementation to the
// review trait; it makes no timing decisions and buffers no packets.
struct ExactMp4(zenmp4::Mp4Muxer);
impl Muxer for ExactMp4 {
    type Error = zenmp4::Mp4Error;
    fn begin(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<Self::Error>> {
        self.0.begin(tracks, hint)
    }
    fn begin_exact_timing(
        &mut self,
        tracks: &[TrackInfo],
        hint: MuxHint,
    ) -> Result<MuxTiming, SessionError<Self::Error>> {
        self.0.begin_exact_timing(tracks, hint)
    }
    fn write_packet(
        &mut self,
        packet: Packet,
        stop: Option<&dyn zencodec::Pulse>,
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
    fn finish(
        self,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<MuxFinish, SessionError<Self::Error>> {
        self.0.finish(stop)
    }
}

fn write_exact<D: Destination>(destination: D, info: &TrackInfo, packets: &[Packet]) -> Stats {
    let mut output = Output::new(
        ExactMp4(zenmp4::Mp4Muxer::new()),
        destination,
        std::slice::from_ref(info),
        Some(&NoPulse),
    )
    .unwrap();
    for packet in packets {
        output.send(packet.clone(), Some(&NoPulse)).unwrap();
    }
    output.finish(Some(&NoPulse)).unwrap()
}

#[test]
fn mux_native_exact_timing_preserves_variable_durations_decode_clocks_and_origins() {
    for bits in [8, 10] {
        let settings = settings(bits, Subsampling::Yuv420, false);
        let frames: Vec<_> = (0..6).map(|i| frame(&settings, i)).collect();
        let (config, original) = encode(settings.clone(), &frames);
        for scale in [1, 90] {
            let base = TimeBase::new(1, 1000 * scale).unwrap();
            let mut config = config.clone();
            config.time_base = base;
            for origin in [0, 40] {
                let mut dts = origin;
                let packets: Vec<_> = original
                    .iter()
                    .zip([17, 23, 40, 1, 70, 9])
                    .map(|(p, duration)| {
                        let packet = p
                            .clone()
                            .with_pts(zencodec::time::Timestamp::new(
                                (dts + 3) * i64::from(scale),
                                base,
                            ))
                            .with_dts(zencodec::time::Timestamp::new(dts * i64::from(scale), base))
                            .with_duration_ticks(duration * u64::from(scale));
                        dts += duration as i64;
                        packet
                    })
                    .collect();
                for forward in [false, true] {
                    let name = format!("mux-exact-{bits}-{scale}-{origin}-{forward}");
                    let mut bytes = Vec::new();
                    let stats = if forward {
                        write_exact(
                            Forward(Short {
                                writer: &mut bytes,
                                interrupted: false,
                            }),
                            &track(&config),
                            &packets,
                        )
                    } else {
                        let path = root().join(format!("{name}.embedded"));
                        let mut file = OpenOptions::new()
                            .create_new(true)
                            .write(true)
                            .read(true)
                            .open(&path)
                            .unwrap();
                        let prefix = b"prior content";
                        file.write_all(prefix).unwrap();
                        let stats = write_exact(
                            SeekBack::new(Short {
                                writer: &mut file,
                                interrupted: false,
                            })
                            .unwrap(),
                            &track(&config),
                            &packets,
                        );
                        assert_eq!(
                            file.stream_position().unwrap(),
                            prefix.len() as u64 + stats.emitted
                        );
                        drop(file);
                        let embedded = std::fs::read(path).unwrap();
                        assert_eq!(&embedded[..prefix.len()], prefix);
                        bytes.extend_from_slice(&embedded[prefix.len()..]);
                        stats
                    };
                    assert_eq!(stats.patches, 0);
                    oracle_container(&root(), &name, "mp4", &settings, &bytes, &frames);
                    assert_eq!(inspect_timing(&name, "mp4", &packets), packets.len());
                    let inspection =
                        std::fs::read_to_string(root().join(&name).join("ffprobe.stdout")).unwrap();
                    let clock = inspection
                        .lines()
                        .find_map(|l| l.strip_prefix("stream|time_base="))
                        .unwrap();
                    let (n, d) = clock.split_once('/').unwrap();
                    let (n, d): (i128, i128) = (n.parse().unwrap(), d.parse().unwrap());
                    for (line, packet) in inspection
                        .lines()
                        .filter(|l| l.starts_with("packet|"))
                        .zip(&packets)
                    {
                        let actual: i128 = line
                            .split('|')
                            .find_map(|p| p.strip_prefix("dts="))
                            .unwrap()
                            .parse()
                            .unwrap();
                        let expected = packet.dts().unwrap();
                        assert_eq!(
                            actual * n * i128::from(base.denominator()),
                            i128::from(expected.ticks()) * d
                        );
                    }
                    std::fs::write(
                        root().join(&name).join("output-counts.txt"),
                        format!(
                            "emitted={} patches={} max_pending={}\n",
                            stats.emitted, stats.patches, stats.max_pending
                        ),
                    )
                    .unwrap();
                }
            }
        }
    }
}

#[test]
fn mux_native_exact_timing_packet_refusal_is_terminal_with_prior_progress() {
    let settings = settings(8, Subsampling::Yuv420, false);
    let frames: Vec<_> = (0..2).map(|i| frame(&settings, i)).collect();
    let (config, packets) = encode(settings, &frames);
    for invalid in [
        Packet::from_vec(packets[0].bytes().to_vec())
            .unwrap()
            .with_pts(packets[0].pts().unwrap()),
        packets[0]
            .clone()
            .with_dts(zencodec::time::Timestamp::new(-1, config.time_base)),
        packets[0].clone().with_duration_ticks(u64::MAX),
    ] {
        let mut bytes = Vec::new();
        let mut output = Output::new(
            ExactMp4(zenmp4::Mp4Muxer::new()),
            Forward(&mut bytes),
            &[track(&config)],
            Some(&NoPulse),
        )
        .unwrap();
        let before = output.stats().emitted;
        let failure = output.send(invalid, Some(&NoPulse)).unwrap_err();
        assert!(matches!(failure.stage, Stage::Packet));
        assert_eq!(failure.emitted, before);
        assert!(zencodec::find_cause::<SessionError<zenmp4::Mp4Error>>(&failure).is_some());
        assert!(output.is_failed());
        assert!(output.send(packets[0].clone(), Some(&NoPulse)).is_err());
        drop(output);
        assert_eq!(bytes.len() as u64, before);
    }
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
        let mut output =
            Output::with_container_timing(make(), destination, &[track], Some(&NoPulse)).unwrap();
        let facts = output.facts();
        assert!(!facts.seek_back);
        assert!(!facts.exact_timing);
        assert!(!facts.patches_at_finish);

        for packet in packets {
            output.send(packet, Some(&NoPulse)).unwrap();
        }
        let stats = output.finish(Some(&NoPulse)).unwrap();
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
        let mut output =
            Output::with_container_timing(make(), destination, &[track], Some(&NoPulse)).unwrap();
        let facts = output.facts();
        assert!(facts.seek_back);
        assert!(!facts.exact_timing);
        assert_eq!(facts.patches_at_finish, extension != "webm");

        for packet in packets {
            output.send(packet, Some(&NoPulse)).unwrap();
        }
        let stats = output.finish(Some(&NoPulse)).unwrap();
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
        let mut output = Output::with_container_timing(
            make(),
            Forward(&mut bytes),
            &[track(&config)],
            Some(&NoPulse),
        )
        .unwrap();
        let before = output.stats().emitted;
        let bad = if epoch {
            packets[0].clone().with_config_epoch(config.epoch + 1)
        } else {
            packets[0].clone().with_pts(zencodec::time::Timestamp::new(
                0,
                TimeBase::hz(90_000).unwrap(),
            ))
        };
        let failure = output.send(bad, Some(&NoPulse)).unwrap_err();
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
        assert!(output.send(packets[1].clone(), Some(&NoPulse)).is_err());
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
    let error = match Output::with_container_timing(
        make(),
        Forward(Broken {
            bytes: &mut bytes,
            remaining: 13,
        }),
        &[track(&config)],
        Some(&NoPulse),
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
        stop: Option<&dyn zencodec::Pulse>,
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
    fn finish(
        self,
        stop: Option<&dyn zencodec::Pulse>,
    ) -> Result<MuxFinish, SessionError<Self::Error>> {
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
    let mut output = Output::with_container_timing(
        BadFinish(zenmp4::Mp4Muxer::new()),
        SeekBack::new(&mut file).unwrap(),
        &[track(&config)],
        Some(&NoPulse),
    )
    .unwrap();
    for packet in packets {
        output.send(packet, Some(&NoPulse)).unwrap();
    }
    let before = std::fs::read(&path).unwrap();
    let failure = output.finish(Some(&NoPulse)).unwrap_err();
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
        let mut output = Output::with_container_timing(
            make(),
            Forward(&mut writer),
            &[track(&config)],
            Some(&NoPulse),
        )
        .unwrap();
        for packet in &packets {
            output.send(packet.clone(), Some(&NoPulse)).unwrap();
        }
        output.finish(Some(&NoPulse))
    } else {
        let mut output = Output::with_container_timing(
            make(),
            SeekBack::new(&mut writer).unwrap(),
            &[track(&config)],
            Some(&NoPulse),
        )
        .unwrap();
        for packet in &packets {
            output.send(packet.clone(), Some(&NoPulse)).unwrap();
        }
        output.finish(Some(&NoPulse))
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
impl enough::Stop for StopAfterWrite {
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
        let mut output = Output::with_container_timing(
            zenivf::IvfMuxer::new(),
            Forward(TrippingWriter {
                bytes: &mut bytes,
                stop: &stop,
            }),
            &[track(&config)],
            Some(&zencodec::how_far::StopOnly::borrowed(&stop)),
        )
        .unwrap();
        let header = output.stats().emitted;
        stop.armed.store(true, std::sync::atomic::Ordering::SeqCst);
        let error = output
            .send(
                packets[0].clone(),
                Some(&zencodec::how_far::StopOnly::borrowed(&stop)),
            )
            .unwrap_err();
        assert_eq!(error.emitted, header + 7);
        assert!(
            matches!(zencodec::find_cause::<SessionError<ErasedError>>(&error), Some(SessionError::Cancelled(actual)) if *actual == reason)
        );
        assert!(output.is_failed());
        assert!(output.send(packets[0].clone(), Some(&NoPulse)).is_err());
        drop(output);
        assert_eq!(bytes.len() as u64, error.emitted);
    }
}

#[test]
fn mux_native_coupled_retimed_encoder_packets_preserve_exact_mp4_timing() {
    use crate::epoch_route::Candidate;
    use std::{
        cell::Cell,
        sync::{Arc, atomic::AtomicUsize},
    };
    use zencodec::{
        At,
        frame::FrameTiming,
        operation::{Decision, FrameConversionOffer, FrameFacts, Question},
        storage::StorageBudget,
        time::Timestamp,
    };
    let settings = settings(8, Subsampling::Yuv420, false);
    let frames: Vec<_> = (0..3).map(|i| frame(&settings, i)).collect();
    let (config, packets) = encode(settings, &frames);
    let inputs = decode(
        &mut zenavif::media::Av1VideoDecoder::new().unwrap(),
        &config,
        &packets,
        Some(&NoPulse),
    )
    .unwrap();
    let clock = TimeBase::hz(90_000).unwrap();
    let timings = [(9000, 1800), (10800, 4500), (15300, 2700)].map(|(pts, duration)| {
        FrameTiming::at(Timestamp::new(pts, clock)).with_duration_ticks(duration)
    });
    let calls = Cell::new(0usize);
    let preparations = Cell::new(0usize);
    let storage = StorageBudget::new(1 << 20, 64);
    let live = Arc::new(AtomicUsize::new(0));
    let mut operation = Candidate::new(
        |q: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
            let Question::FrameRoute { facts, choices, .. } = q else {
                panic!()
            };
            let i = calls.get();
            assert_eq!(preparations.get(), 1);
            assert_eq!(facts.input.timing, inputs[i].timing());
            assert_eq!(facts.output.output_committed, i > 0);
            let choice = choices.first().unwrap();
            let destination = choice.value().destination.unwrap();
            assert!(!destination.seek_back);
            assert!(destination.streams);
            assert!(!destination.patches_at_finish);
            assert!(destination.exact_timing);
            assert_eq!(
                choice.value().conversion.unwrap().encoder_input.timing,
                timings[i]
            );
            calls.set(i + 1);
            Ok(Decision::choose(choice))
        },
        false,
        false,
        storage.clone(),
        live.clone(),
    );
    let mut bytes = Vec::new();
    let mut writer = Some(&mut bytes);
    let mut destination: Option<Output<ExactMp4, Forward<Short<&mut Vec<u8>>>>> = None;
    let mut expected = Vec::new();
    for (i, input) in inputs.iter().enumerate() {
        let mut offer =
            FrameConversionOffer::new(FrameFacts::new(input.interpretation(), timings[i]));
        offer.preserves_samples = true;
        offer.preserves_extended_range = true;
        let prepared = operation
            .push_route_with_destination(
                input,
                (input.interpretation(), timings[i]),
                Some(offer),
                |config| {
                    if i == 0 {
                        let output = PreparedOutput::new(
                            ExactMp4(zenmp4::Mp4Muxer::new()),
                            Forward(Short {
                                writer: writer.take().unwrap(),
                                interrupted: false,
                            }),
                            &[track(config)],
                            Some(&NoPulse),
                        )?;
                        assert!(!output.timing().patches_at_finish());
                        preparations.set(preparations.get() + 1);
                        let facts = output.facts();
                        Ok((Some(output), Some(facts)))
                    } else {
                        assert!(destination.is_some());
                        Ok((None, Some(destination.as_ref().unwrap().facts())))
                    }
                },
                || {
                    let output = input.represent(timings[i], input.presentation());
                    expected.push(output.clone());
                    Ok(output)
                },
                Some(&NoPulse),
            )
            .unwrap();
        if i == 0 {
            operation
                .publish_header(
                    |_| {
                        destination = Some(prepared.unwrap().publish(Some(&NoPulse))?);
                        Ok(())
                    },
                    Some(&NoPulse),
                )
                .unwrap();
        }
    }
    for record in operation.report().decisions() {
        let Some(zencodec::operation::Selection::FrameRoute(offer)) = record.selected else {
            panic!()
        };
        assert!(offer.destination.unwrap().exact_timing);
        assert!(!offer.destination.unwrap().seek_back);
    }
    let segments = operation.finish(Some(&NoPulse)).unwrap();
    assert_eq!(segments.len(), 1);
    assert_eq!(live.load(std::sync::atomic::Ordering::SeqCst), 0);
    let segment = &segments[0];
    let mut destination = destination.unwrap();
    for (packet, timing) in segment.packets.iter().zip(timings) {
        assert_eq!(packet.pts(), timing.pts);
        assert_eq!(packet.duration_ticks(), timing.duration_ticks);
        // No timestamp rewriting between native encoder and native muxer.
        destination.send(packet.clone(), Some(&NoPulse)).unwrap();
    }
    let stats = destination.finish(Some(&NoPulse)).unwrap();
    assert_eq!(stats.patches, 0);
    let name = "mux-coupled-retimed";
    oracle_container(&root(), name, "mp4", &segment.settings, &bytes, &expected);
    assert_eq!(inspect_timing(name, "mp4", &segment.packets), timings.len());
    drop(segments);
    assert_eq!(storage.reserved_bytes(), 0);
}

#[test]
fn mux_native_coupled_partial_header_failure_terminates_encoder_and_retains_cause() {
    use crate::epoch_route::Candidate;
    use std::sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    };
    use zencodec::{operation::PolicySet, storage::StorageBudget};
    let input = frame(&settings(8, Subsampling::Yuv420, false), 0);
    let storage = StorageBudget::new(1 << 20, 64);
    let live = Arc::new(AtomicUsize::new(0));
    let mut operation = Candidate::new(
        PolicySet::new(),
        false,
        false,
        storage.clone(),
        live.clone(),
    );
    operation.push(&input, Some(&NoPulse)).unwrap();
    assert_eq!(live.load(Ordering::SeqCst), 1);
    let mut bytes = Vec::new();
    let failure = operation
        .publish_header(
            |config| {
                let _output = Output::new(
                    ExactMp4(zenmp4::Mp4Muxer::new()),
                    Forward(Broken {
                        bytes: &mut bytes,
                        remaining: 13,
                    }),
                    &[track(config)],
                    Some(&NoPulse),
                )?;
                Ok(())
            },
            Some(&NoPulse),
        )
        .unwrap_err();
    let mux = zencodec::find_cause::<Failure>(&*failure).unwrap();
    assert!(matches!(mux.stage, Stage::Drain));
    assert_eq!(mux.emitted, 13);
    assert_eq!(
        zencodec::find_cause::<io::Error>(&*failure).unwrap().kind(),
        io::ErrorKind::BrokenPipe
    );
    assert_eq!(bytes.len(), 13);
    assert!(operation.is_failed());
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert_eq!(storage.reserved_bytes(), 0);
    assert!(operation.push(&input, Some(&NoPulse)).is_err());
    assert!(
        operation
            .publish_header(
                |_| panic!("terminal operation retried publication"),
                Some(&NoPulse)
            )
            .is_err()
    );
    assert_eq!(bytes.len(), 13);
}

#[test]
fn mux_native_coupled_destination_refusal_precedes_policy_and_conversion() {
    use crate::epoch_route::Candidate;
    use std::{
        cell::Cell,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use zencodec::{
        At,
        operation::{Decision, Question},
        storage::StorageBudget,
    };
    let input = frame(&settings(8, Subsampling::Yuv420, false), 0);
    let calls = Cell::new(0usize);
    let live = Arc::new(AtomicUsize::new(0));
    let storage = StorageBudget::new(1 << 20, 64);
    let mut operation = Candidate::new(
        |_: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
            calls.set(calls.get() + 1);
            Ok(Decision::UsePreset)
        },
        false,
        false,
        storage.clone(),
        live.clone(),
    );
    let mut bytes = Vec::new();
    let error = match operation.push_route_with_destination(
        &input,
        (input.interpretation(), input.timing()),
        None,
        |config| {
            let output = PreparedOutput::new(
                zenivf::IvfMuxer::new(),
                Forward(&mut bytes),
                &[track(config)],
                Some(&NoPulse),
            )?;
            let facts = output.facts();
            Ok((output, Some(facts)))
        },
        || panic!("unsupported destination executed conversion"),
        Some(&NoPulse),
    ) {
        Ok(_) => panic!("unqualified exact timing accepted"),
        Err(error) => error,
    };
    assert!(matches!(
        zencodec::find_cause::<SessionError<zenivf::IvfError>>(&*error),
        Some(SessionError::Unsupported(UnsupportedMedia::ExactTiming))
    ));
    assert_eq!(calls.get(), 0);
    assert!(bytes.is_empty());
    assert!(operation.is_failed());
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert_eq!(storage.reserved_bytes(), 0);
}

#[test]
fn mux_native_coupled_policy_refusal_drops_prepared_destination_without_publication() {
    use crate::epoch_route::Candidate;
    use std::{
        cell::Cell,
        sync::{
            Arc,
            atomic::{AtomicUsize, Ordering},
        },
    };
    use zencodec::{
        At,
        operation::{Decision, Decline, PolicyFailure, Question},
        storage::StorageBudget,
    };
    let input = frame(&settings(8, Subsampling::Yuv420, false), 0);
    let prepared = Cell::new(false);
    let live = Arc::new(AtomicUsize::new(0));
    let storage = StorageBudget::new(1 << 20, 64);
    let mut operation = Candidate::new(
        |q: Question<'_>| -> Result<Decision, At<std::convert::Infallible>> {
            assert!(prepared.get());
            let Question::FrameRoute { choices, .. } = q else {
                panic!()
            };
            let destination = choices.first().unwrap().value().destination.unwrap();
            assert!(destination.exact_timing);
            assert!(!destination.seek_back);
            assert!(!destination.patches_at_finish);
            Ok(Decision::decline(Decline::NoPreferredChoice))
        },
        false,
        false,
        storage.clone(),
        live.clone(),
    );
    let mut bytes = Vec::new();
    let error = match operation.push_route_with_destination(
        &input,
        (input.interpretation(), input.timing()),
        None,
        |config| {
            let output = PreparedOutput::new(
                ExactMp4(zenmp4::Mp4Muxer::new()),
                Forward(&mut bytes),
                &[track(config)],
                Some(&NoPulse),
            )?;
            prepared.set(true);
            let facts = output.facts();
            Ok((output, Some(facts)))
        },
        || panic!("declined route executed conversion"),
        Some(&NoPulse),
    ) {
        Ok(_) => panic!("declined route accepted"),
        Err(error) => error,
    };
    assert!(zencodec::find_cause::<PolicyFailure>(&*error).is_some());
    assert!(bytes.is_empty());
    assert!(operation.is_failed());
    assert_eq!(live.load(Ordering::SeqCst), 0);
    assert_eq!(storage.reserved_bytes(), 0);
}

#[path = "staged_encode_tests.rs"]
mod staged_encode;

// A qualification adapter: one packed raster becomes one native AV1 frame in
// IVF. It buffers the raster explicitly; this tests patch transport, not bounded
// encoding or an efficient native raster implementation.
struct RasterIvf {
    settings: zenavif::media::Av1EncodeSettings,
    bytes: Vec<u8>,
    seek: bool,
}
impl zencodec::encode::Encoder for RasterIvf {
    type Error = io::Error;
    fn reject(op: zencodec::UnsupportedOperation) -> Self::Error {
        io::Error::other(op)
    }
    fn encode(
        self,
        _: zenpixels::PixelSlice<'_>,
    ) -> Result<zencodec::encode::EncodeOutput, Self::Error> {
        Err(io::Error::other("qualification uses negotiated rows"))
    }
    fn begin_output(
        &mut self,
        w: u32,
        h: u32,
        desc: zenpixels::PixelDescriptor,
        access: zencodec::output_patch::OutputAccess,
    ) -> Result<zencodec::encode::OutputTiming, Self::Error> {
        if w != self.settings.width
            || h != self.settings.height
            || desc != zenpixels::PixelDescriptor::RGB8_SRGB
        {
            return Err(io::Error::other("unexpected raster declaration"));
        }
        self.seek = access == zencodec::output_patch::OutputAccess::SeekBack;
        Ok(if self.seek {
            zencodec::encode::OutputTiming::AT_FINISH.with_patches()
        } else {
            zencodec::encode::OutputTiming::AT_FINISH
        })
    }
    fn push_rows(&mut self, rows: zenpixels::PixelSlice<'_>) -> Result<(), Self::Error> {
        for y in 0..rows.rows() {
            self.bytes.extend_from_slice(rows.row(y));
        }
        Ok(())
    }
    fn finish_output(
        self,
    ) -> Result<zencodec::output_patch::OutputFinish<zencodec::encode::EncodeOutput>, Self::Error>
    {
        let frame = rgb_frame(&self.settings, &self.bytes);
        let (config, packets) = encode(self.settings, &[frame]);
        let mut mux = zenivf::IvfMuxer::new().with_count_patch();
        mux.begin(
            &[track(&config)],
            if self.seek {
                MuxHint::default()
            } else {
                MuxHint::forward_only()
            },
        )
        .map_err(io::Error::other)?;
        let mut bytes = Vec::new();
        mux.take_output(&mut bytes);
        for packet in packets {
            mux.write_packet(packet, Some(&NoPulse))
                .map_err(io::Error::other)?;
            mux.take_output(&mut bytes);
        }
        let finish = mux.finish(Some(&NoPulse)).map_err(io::Error::other)?;
        bytes.extend_from_slice(&finish.tail);
        Ok(zencodec::output_patch::OutputFinish::new(
            zencodec::encode::EncodeOutput::new(bytes, zencodec::ImageFormat::Unknown)
                .with_extension("ivf")
                .with_mime_type("video/x-ivf"),
            finish.patches,
        ))
    }
}
fn rgb_frame(s: &zenavif::media::Av1EncodeSettings, rgb: &[u8]) -> zencodec_media::frame::Frame {
    use std::sync::Arc;
    use zencodec_media::frame::*;
    assert_eq!(rgb.len(), s.width as usize * s.height as usize * 3);
    let encoding = SampleEncoding::new(zenpixels::ChannelType::U8, 8, 0).unwrap();
    let planes = [1, 2, 0]
        .into_iter()
        .map(|c| OwnedPlane {
            data: PlaneData::U8(rgb.as_chunks::<3>().0.iter().map(|p| p[c]).collect()),
            width: s.width as usize,
            height: s.height as usize,
            stride_bytes: s.width as usize,
        })
        .collect();
    let mut interpretation = FrameInterpretation::new(
        FrameGeometry::full(s.width, s.height),
        Subsampling::Yuv444,
        s.siting,
        encoding,
        vec![PlaneRole::Green, PlaneRole::Blue, PlaneRole::Red],
    );
    interpretation.color = zencodec::frame::ColorClaim::new(None, None, Some(s.color));
    Frame::new(
        Arc::new(OwnedPlanes::new(planes, encoding).unwrap()),
        Arc::new(interpretation),
        FrameTiming::at(zencodec::time::Timestamp::new(0, TB)).with_duration_ticks(40),
        Presentation::default(),
    )
}
#[test]
fn mux_native_raster_destination_negotiates_and_applies_real_ivf_count_patch() {
    use zencodec::{operation::PolicySet, storage::StorageBudget};
    use zencodec_io::row_encode::{self, Options, Raster};
    let s = settings(8, Subsampling::Yuv444, true);
    let stride = s.width as usize * 3 + 6;
    let mut data = vec![0xa5; stride * s.height as usize];
    let mut packed = Vec::new();
    for y in 0..s.height as usize {
        for x in 0..s.width as usize * 3 {
            data[y * stride + x] = (x * 19 + y * 37) as u8;
            packed.push(data[y * stride + x]);
        }
    }
    let expected = rgb_frame(&s, &packed);
    let view = || {
        zenpixels::PixelSlice::new(
            &data,
            s.width,
            s.height,
            stride,
            zenpixels::PixelDescriptor::RGB8_SRGB,
        )
        .unwrap()
    };
    let options = || Options::new(StorageBudget::new(1 << 20, 1024), 1 << 20);
    let encoder = || RasterIvf {
        settings: s.clone(),
        bytes: Vec::new(),
        seek: false,
    };
    let mut forward = Vec::new();
    let result = row_encode::encode(
        encoder(),
        Raster::new(view()),
        &mut forward,
        PolicySet::new(),
        options(),
        Some(&NoPulse),
    )
    .unwrap();
    assert_eq!(result.patched_bytes, 0);
    let mut disk = io::Cursor::new(b"prefix".to_vec());
    disk.set_position(6);
    let result = row_encode::encode_to_destination(
        encoder(),
        Raster::new(view()),
        SeekBack::new(&mut disk).unwrap(),
        PolicySet::new(),
        options(),
        Some(&NoPulse),
    )
    .unwrap();
    assert_eq!(result.patched_bytes, 4);
    assert_eq!(disk.position(), 6 + result.emitted);
    let bytes = disk.into_inner();
    assert_eq!(&bytes[..6], b"prefix");
    assert_eq!(u32::from_le_bytes(bytes[30..34].try_into().unwrap()), 1);
    oracle_container(
        &root(),
        "raster-forward",
        "ivf",
        &s,
        &forward,
        std::slice::from_ref(&expected),
    );
    oracle_container(&root(), "raster-seek", "ivf", &s, &bytes[6..], &[expected]);
}
