//! Admission follows real packet and picture owners across queue/epoch changes.
use std::sync::Arc;
use zencodec_io::storage::{StorageBudget, StorageError};
use zencodec_media::{
    frame::{
        ChromaSiting, Frame, FrameGeometry, FrameInterpretation, FrameTiming, OwnedPlane,
        OwnedPlanes, PlaneData, PlaneRole, PlaneSamples, Presentation, SampleEncoding, Subsampling,
    },
    packet::{Packet, PacketAddition, PacketPayload},
};

#[test]
fn deferred_admission_blocks_competitors_and_transfers_the_same_charge() {
    let pool = StorageBudget::new(24, 1);
    let prepared = pool.prepare_filled(4, [0x1234u16; 3]).unwrap();
    assert_eq!(prepared.reserved_bytes(), 24);
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (24, 1));
    assert!(matches!(pool.bytes(0), Err(StorageError::Objects { .. })));
    let data = prepared.allocate().unwrap();
    assert_eq!(data.as_slice(), &[[0x1234; 3]; 4]);
    let shared = data.freeze();
    let empty = shared.slice(2..2).unwrap();
    drop(shared);
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (24, 1));
    drop(empty);
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
}

#[test]
fn deferred_size_errors_and_unwind_release_all_allowances() {
    let pool = StorageBudget::new(24, 2);
    assert!(matches!(
        pool.prepare_filled(usize::MAX, 1u16),
        Err(StorageError::SizeOverflow)
    ));
    assert!(matches!(
        pool.prepare_filled(25, 0u8),
        Err(StorageError::Bytes { .. })
    ));
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
    let unwind = std::panic::catch_unwind(|| {
        let _held = pool.prepare_filled(24, 0u8).unwrap();
        assert!(matches!(
            pool.prepare_filled(1, 0u8),
            Err(StorageError::Bytes { .. })
        ));
        panic!("application callback unwound");
    });
    assert!(unwind.is_err());
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
    let empty = pool.prepare_filled(0, 0u8).unwrap().allocate().unwrap();
    let zero_sized = pool.prepare_filled(7, ()).unwrap().allocate().unwrap();
    assert!(empty.is_empty());
    assert_eq!(zero_sized.len(), 7);
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 2));
    drop((empty, zero_sized));
    assert_eq!(pool.reserved_objects(), 0);
}

#[test]
#[cfg(target_pointer_width = "64")]
fn ten_gib_reservation_can_be_declined_without_materializing_pixels() {
    let bytes = 10usize * 1024 * 1024 * 1024;
    let pool = StorageBudget::new(bytes, 1);
    let proposed = pool.prepare_filled(bytes, 0u8).unwrap();
    assert_eq!(proposed.reserved_bytes(), bytes);
    drop(proposed);
    assert_eq!((pool.reserved_bytes(), pool.reserved_objects()), (0, 0));
    assert_eq!(pool.bytes(1).unwrap().as_slice(), &[0]);
}

fn frame(data: PlaneData, encoding: SampleEncoding, epoch: u32) -> Frame {
    let words = encoding.bytes_per_sample();
    let picture = OwnedPlanes::new(
        vec![OwnedPlane {
            data,
            width: 3,
            height: 2,
            stride_bytes: 4 * words,
        }],
        encoding,
    )
    .unwrap();
    let interpretation = FrameInterpretation::new(
        FrameGeometry::full(3, 2),
        Subsampling::Mono,
        ChromaSiting::Unknown,
        encoding,
        vec![PlaneRole::Luma],
    )
    .with_config_epoch(epoch);
    Frame::new(
        Arc::new(picture),
        Arc::new(interpretation),
        FrameTiming::default(),
        Presentation::default(),
    )
}

#[test]
fn packet_take_side_data_and_clones_share_one_backing_charge() {
    let budget = StorageBudget::new(32, 1);
    let mut bytes = budget.bytes(32).unwrap();
    for (i, byte) in bytes.as_mut_slice().iter_mut().enumerate() {
        *byte = i as u8;
    }
    let shared = bytes.freeze();
    let mut packet = Packet::from_accounted(shared.slice(8..16).unwrap()).unwrap();
    let addition = PacketAddition::new(1, PacketPayload::Accounted(shared.slice(16..20).unwrap()));
    let clone = packet.clone();
    drop(shared);
    let taken = packet.take().unwrap();
    assert_eq!(taken.as_bytes(), &[8, 9, 10, 11, 12, 13, 14, 15]);
    drop(packet);
    drop(clone);
    drop(taken);
    assert_eq!(
        budget.reserved_bytes(),
        32,
        "side data still owns the allocation"
    );
    assert_eq!(addition.payload().as_bytes(), &[16, 17, 18, 19]);
    drop(addition);
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
}

#[test]
fn retained_old_epoch_frames_remain_charged_after_queue_and_decoder_handles_drop() {
    let budget = StorageBudget::new(16, 2);
    let mut old = budget.bytes(8).unwrap();
    old.as_mut_slice()
        .copy_from_slice(&[1, 2, 3, 99, 4, 5, 6, 98]);
    let old = frame(PlaneData::AccountedU8(old.freeze()), SampleEncoding::U8, 0);
    let retained = old.clone();
    let replay = old.represent(FrameTiming::default(), Presentation::default());
    assert!(old.shares_picture(&replay));
    drop(old); // decoder/queue released its handle
    let new = frame(
        PlaneData::AccountedU8(budget.bytes(8).unwrap().freeze()),
        SampleEncoding::U8,
        1,
    );
    assert_eq!(budget.reserved_bytes(), 16);
    assert!(matches!(budget.bytes(1), Err(StorageError::Objects { .. })));
    assert_eq!(retained.interpretation().config_epoch, 0);
    assert_eq!(new.interpretation().config_epoch, 1);
    let mapped = retained.map().unwrap();
    let plane = mapped.plane(0).unwrap();
    assert!(matches!(plane.row(0).unwrap(), PlaneSamples::U8(row) if row == [1, 2, 3]));
    assert!(matches!(plane.row(1).unwrap(), PlaneSamples::U8(row) if row == [4, 5, 6]));
    drop(mapped);
    drop(retained);
    assert_eq!(
        budget.reserved_bytes(),
        16,
        "show-existing presentation still owns old pixels"
    );
    drop(replay);
    assert_eq!(budget.reserved_bytes(), 8);
    drop(new);
    assert_eq!((budget.reserved_bytes(), budget.reserved_objects()), (0, 0));
}

#[test]
fn ten_bit_strided_planes_keep_native_words_and_their_full_charge() {
    let budget = StorageBudget::new(16, 1);
    let mut words = budget.words(8).unwrap();
    words
        .as_mut_slice()
        .copy_from_slice(&[0, 512, 1023, 65535, 1, 511, 1022, 65534]);
    let encoding = SampleEncoding::new(zenpixels::ChannelType::U16, 10, 0).unwrap();
    let frame = frame(PlaneData::AccountedU16(words.freeze()), encoding, 5);
    let clone = frame.clone();
    drop(frame);
    assert_eq!(budget.reserved_bytes(), 16);
    let mapped = clone.map().unwrap();
    let plane = mapped.plane(0).unwrap();
    assert!(matches!(plane.row(0).unwrap(), PlaneSamples::U16(row) if row == [0, 512, 1023]));
    assert!(matches!(plane.row(1).unwrap(), PlaneSamples::U16(row) if row == [1, 511, 1022]));
    assert_eq!(clone.interpretation().encoding, encoding);
    drop(mapped);
    drop(clone);
    assert_eq!(budget.reserved_bytes(), 0);
}
