use super::*;
use crate::{
    checked_collection::Checked,
    file_snapshot::{self, clone_source},
    image_items::{Composition, ImageId, ImageRequest},
    tiff_items::TiffCollection,
};
use std::fs::File;
use zencodec::{SharedPulse, NoPulse, operation::PolicySet};
use zencodec_io::FileReadAt;

type Disk = Checked<TiffCollection<BufReader<ReadAtReader<FileReadAt>>>, PolicySet>;
fn directory(name: &str) -> PathBuf {
    let path = artifacts().join(name);
    std::fs::create_dir(&path).unwrap();
    path
}
fn create(path: &Path, data: &[u8]) -> File {
    let mut file = File::options()
        .create_new(true)
        .read(true)
        .write(true)
        .open(path)
        .unwrap();
    file.write_all(data).unwrap();
    file
}
fn open(source: FileReadAt, buffer: usize) -> Disk {
    Checked::open(SharedPulse::default(), PolicySet::new(), |stop| {
        TiffCollection::open_with_stop(
            BufReader::with_capacity(buffer, ReadAtReader::new(source)),
            Composition::Composited,
            Allowances::default(),
            stop,
        )
        .map_err(Into::into)
    })
    .unwrap()
}
fn child(collection: &mut Disk) -> ImageId {
    let mut roots = collection.images().unwrap();
    let root = collection.next(&mut roots).unwrap().unwrap();
    let mut children = collection.children(&root).unwrap();
    collection.next(&mut children).unwrap().unwrap()
}
fn assert_pixels(collection: &mut Disk, id: &ImageId, expected: &Raster) {
    let output = collection.decode(id, &ImageRequest::default()).unwrap();
    output
        .with_pixels(|p| assert_eq!(p.as_contiguous_bytes().unwrap(), expected.bytes))
        .unwrap();
    assert_eq!(output.report().decisions().count(), 1);
}

#[test]
fn file_snapshot_open_handle_keeps_native_subifds_across_atomic_path_replacement() {
    let root = directory("rename");
    for big in [false, true] {
        for planar in [false, true] {
            for buffer in [0, 4096] {
                let (original, old) = fixture_version(big, planar, false, 0);
                let (replacement, new) = fixture_version(big, planar, false, 103);
                assert_ne!(old[1].bytes, new[1].bytes);
                let path = root.join(format!("source-{big}-{planar}-{buffer}.tif"));
                let next = path.with_extension("next.tif");
                create(&path, &original);
                create(&next, &replacement);
                let mut pinned = open(FileReadAt::open(&path).unwrap(), buffer);
                let id = child(&mut pinned);
                std::fs::rename(&next, &path).unwrap();
                assert_pixels(&mut pinned, &id, &old[1]);
                let mut current = open(FileReadAt::open(&path).unwrap(), buffer);
                let id = child(&mut current);
                assert_pixels(&mut current, &id, &new[1]);
            }
        }
    }
}

#[test]
fn file_snapshot_open_handle_alone_does_not_prevent_in_place_source_changes() {
    let root = directory("mutable-handle-control");
    for big in [false, true] {
        for planar in [false, true] {
            let (original, old) = fixture_version(big, planar, false, 0);
            let (replacement, new) = fixture_version(big, planar, false, 103);
            assert_eq!(original.len(), replacement.len());
            assert_ne!(old[1].bytes, new[1].bytes);
            let path = root.join(format!("source-{big}-{planar}.tif"));
            let mut writer = create(&path, &original);
            let mut pinned = open(FileReadAt::open(&path).unwrap(), 0);
            let id = child(&mut pinned);
            writer.rewind().unwrap();
            writer.write_all(&replacement).unwrap();
            // Deliberate violation of FileReadAt's caller-stability obligation.
            // Equal-sized native directories remain readable, but returned pixels
            // come from the newer bytes despite discovering the earlier source.
            assert_pixels(&mut pinned, &id, &new[1]);
        }
    }
}

#[test]
fn file_snapshot_atomic_clone_keeps_exact_native_pixels_after_overwrite_and_truncate() {
    let root = directory("clone");
    let spool = File::open(&root).unwrap();
    for big in [false, true] {
        for planar in [false, true] {
            for buffer in [0, 4096] {
                let (original, old) = fixture_version(big, planar, false, 0);
                let (replacement, _) = fixture_version(big, planar, false, 103);
                let path = root.join(format!("source-{big}-{planar}-{buffer}.tif"));
                let mut writer = create(&path, &original);
                let before = std::fs::read_dir(&root).unwrap().count();
                let snapshot =
                    clone_source(&File::open(&path).unwrap(), &spool, original.len() as u64)
                        .unwrap();
                assert_eq!(
                    std::fs::read_dir(&root).unwrap().count(),
                    before,
                    "snapshot exposed a writable name"
                );
                let mut collection = open(snapshot, buffer);
                let id = child(&mut collection);
                writer.rewind().unwrap();
                writer.write_all(&replacement).unwrap();
                assert_pixels(&mut collection, &id, &old[1]);
                writer.set_len(0).unwrap();
                assert_pixels(&mut collection, &id, &old[1]);
                drop(collection);
                assert_eq!(std::fs::read_dir(&root).unwrap().count(), before);
            }
        }
    }
}

#[test]
fn file_snapshot_clone_and_positioned_writer_race_retains_one_stable_representation() {
    use std::{os::unix::fs::FileExt, sync::mpsc, time::Duration};
    let root = directory("concurrent-clone");
    let spool = File::open(&root).unwrap();
    let source = create(&root.join("source.bin"), &[0x24; 4096]);
    let writer = source.try_clone().unwrap();
    let mut held = Vec::new();
    std::thread::scope(|scope| {
        let (start, starts) = mpsc::sync_channel(0);
        let (done, completed) = mpsc::sync_channel(0);
        let worker = scope.spawn(move || {
            let mut round = 0;
            while let Ok(()) = starts.recv() {
                let value = if round % 2 == 0 { 0xc7 } else { 0x24 };
                // One kernel write, not an application transaction spread over
                // write_all retries. Partial writes invalidate this fixture.
                let written = writer.write_at(&[value; 4096], 0);
                if done.send(written).is_err() {
                    return;
                }
                round += 1;
            }
        });
        for _ in 0..32 {
            start.send(()).unwrap();
            let snapshot = clone_source(&source, &spool, 4096);
            assert_eq!(
                completed
                    .recv_timeout(Duration::from_secs(5))
                    .unwrap()
                    .unwrap(),
                4096
            );
            let mut snapshot = snapshot.unwrap();
            let mut bytes = [0u8; 4096];
            snapshot.read_exact_at(0, &mut bytes).unwrap();
            assert!(bytes.iter().all(|v| *v == 0x24) || bytes.iter().all(|v| *v == 0xc7));
            held.push((snapshot, bytes));
        }
        drop(start);
        worker.join().unwrap();
    });
    for (mut snapshot, expected) in held {
        let mut actual = [0u8; 4096];
        snapshot.read_exact_at(0, &mut actual).unwrap();
        assert_eq!(actual, expected);
    }
    assert_eq!(std::fs::read_dir(&root).unwrap().count(), 1);
}

#[test]
fn file_snapshot_required_clone_refuses_limits_invalid_sources_and_bad_descriptors() {
    let root = directory("refusals");
    let spool = File::open(&root).unwrap();
    let path = root.join("source.bin");
    let source = create(&path, b"snapshot");
    assert!(matches!(
        clone_source(&source, &spool, 7),
        Err(file_snapshot::Error::LogicalBytes {
            actual: 8,
            limit: 7
        })
    ));
    assert!(matches!(
        clone_source(&spool, &spool, u64::MAX),
        Err(file_snapshot::Error::NonRegularSource)
    ));
    let error = clone_source(&source, &source, 8).err().unwrap();
    assert!(matches!(error, file_snapshot::Error::Create(_)));
    assert_eq!(
        zencodec::find_cause::<std::io::Error>(&error)
            .unwrap()
            .kind(),
        std::io::ErrorKind::NotADirectory
    );
    let unreadable = File::options().write(true).open(&path).unwrap();
    let error = clone_source(&unreadable, &spool, 8).err().unwrap();
    assert!(matches!(error, file_snapshot::Error::Clone(_)));
    assert_eq!(
        zencodec::find_cause::<std::io::Error>(&error)
            .unwrap()
            .raw_os_error(),
        Some(rustix::io::Errno::BADF.raw_os_error())
    );
    // A real source outside the reflink filesystem must refuse, even though
    // procfs exposes it as a regular file. Do not fall back to reading its bytes.
    let unsupported = File::open("/proc/self/status").unwrap();
    let error = clone_source(&unsupported, &spool, u64::MAX).err().unwrap();
    assert!(matches!(error, file_snapshot::Error::Clone(_)));
    assert!(
        zencodec::find_cause::<std::io::Error>(&error)
            .unwrap()
            .raw_os_error()
            .is_some()
    );
    assert_eq!(
        std::fs::read_dir(&root).unwrap().count(),
        1,
        "failed snapshot left a named file"
    );
}
