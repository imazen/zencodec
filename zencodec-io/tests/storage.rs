use zencodec_io::storage::{StorageBudget, StorageError};

#[test]
fn subviews_keep_the_backing_charge_until_the_last_owner_drops() {
    let budget = StorageBudget::new(32, 2);
    let mut owned = budget.bytes(24).unwrap();
    for (i, byte) in owned.as_mut_slice().iter_mut().enumerate() {
        *byte = i as u8;
    }
    let shared = owned.freeze();
    let tail = shared.slice(20..24).unwrap();
    let nested = tail.slice(1..3).unwrap();
    assert_eq!(nested.as_slice(), &[21, 22]);
    assert_eq!(nested.charged_bytes(), 24);
    assert_eq!(budget.reserved_bytes(), 24);
    assert_eq!(budget.reserved_objects(), 1);
    drop(shared);
    drop(tail);
    assert!(matches!(
        budget.bytes(9),
        Err(StorageError::Bytes {
            requested: 9,
            limit: 32
        })
    ));
    assert_eq!(
        budget.reserved_objects(),
        1,
        "failed byte admission must return its object slot"
    );
    let other = budget.bytes(8).unwrap();
    assert_eq!(budget.reserved_bytes(), 32);
    drop(nested);
    assert_eq!(budget.reserved_bytes(), 8);
    drop(other);
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
}

#[test]
fn typed_storage_charges_words_and_padding_and_never_infers_logical_pixels() {
    let budget = StorageBudget::new(32, 2);
    let mut words = budget.words(8).unwrap();
    words
        .as_mut_slice()
        .copy_from_slice(&[0, 1023, 512, 7, 65535, 2048, 9, 10]);
    let floats = budget.floats(4).unwrap();
    assert_eq!(floats.as_slice(), &[0.0; 4]);
    assert_eq!(budget.reserved_bytes(), 32);
    let words = words.freeze();
    assert_eq!(words.charged_bytes(), 16);
    assert_eq!(words.as_slice(), &[0, 1023, 512, 7, 65535, 2048, 9, 10]);
    drop(floats);
    assert_eq!(budget.reserved_bytes(), 16);
    assert_eq!(words.slice(7..8).unwrap().charged_bytes(), 16);
}

#[test]
fn empty_buffers_use_object_slots_and_invalid_ranges_cannot_escape_a_view() {
    let zero = StorageBudget::new(0, 1);
    let empty = zero.bytes(0).unwrap().freeze();
    assert!(empty.is_empty());
    assert!(matches!(
        zero.bytes(0),
        Err(StorageError::Objects { limit: 1 })
    ));
    assert!(matches!(empty.slice(0..1), Err(StorageError::Range)));
    drop(empty);
    assert_eq!(zero.reserved_objects(), 0);
    let none = StorageBudget::new(usize::MAX, 0);
    assert!(matches!(
        none.bytes(0),
        Err(StorageError::Objects { limit: 0 })
    ));
    let budget = StorageBudget::new(16, 1);
    let root = budget.bytes(16).unwrap().freeze();
    let view = root.slice(4..8).unwrap();
    assert!(matches!(view.slice(0..5), Err(StorageError::Range)));
    let reversed = std::ops::Range { start: 3, end: 2 };
    assert!(matches!(view.slice(reversed), Err(StorageError::Range)));
    let empty = view.slice(4..4).unwrap();
    drop(root);
    drop(view);
    assert_eq!(
        budget.reserved_bytes(),
        16,
        "an empty view still owns its backing storage"
    );
    drop(empty);
    assert_eq!(budget.reserved_bytes(), 0);
}

#[test]
fn size_overflow_and_unwind_release_all_reservations() {
    let budget = StorageBudget::new(usize::MAX, 2);
    assert!(matches!(
        budget.words(usize::MAX),
        Err(StorageError::SizeOverflow)
    ));
    assert!(matches!(
        budget.bytes(usize::MAX),
        Err(StorageError::SizeOverflow)
    ));
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    let caught = std::panic::catch_unwind(|| {
        let _owned = budget.bytes(8).unwrap();
        let _shared = budget.words(4).unwrap().freeze();
        panic!("holding admitted storage");
    });
    assert!(caught.is_err());
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
}

#[test]
fn concurrent_admission_never_exceeds_either_cap_and_releases_losing_reservations() {
    use std::sync::{Arc, Barrier};
    for (max_bytes, max_objects, expected) in [(48, 8, 3), (128, 2, 2)] {
        let budget = StorageBudget::new(max_bytes, max_objects);
        let start = Arc::new(Barrier::new(9));
        let hold = Arc::new(Barrier::new(9));
        let release = Arc::new(Barrier::new(9));
        let mut workers = Vec::new();
        for _ in 0..8 {
            let (budget, start, hold, release) =
                (budget.clone(), start.clone(), hold.clone(), release.clone());
            workers.push(std::thread::spawn(move || {
                start.wait();
                let storage = budget.bytes(16);
                hold.wait();
                release.wait();
                storage.is_ok()
            }));
        }
        start.wait();
        hold.wait();
        assert_eq!(budget.reserved_bytes(), expected * 16);
        assert_eq!(budget.reserved_objects(), expected);
        release.wait();
        assert_eq!(
            workers
                .into_iter()
                .map(|worker| worker.join().unwrap())
                .filter(|ok| *ok)
                .count(),
            expected
        );
        assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
    }
}
