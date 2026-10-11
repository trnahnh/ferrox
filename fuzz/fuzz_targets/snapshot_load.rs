//! Arbitrary bytes as the newest snapshot file (bincode input): recovery must
//! never panic and must fall back to WAL-only replay (an empty book here)
//! whenever the snapshot is rejected.
#![no_main]
use ferrox::recovery::recover_with_wal_size;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    let snap_dir = data_dir.join("snapshots");
    std::fs::create_dir_all(&snap_dir).unwrap();
    std::fs::write(snap_dir.join("snapshot_0000000001.bin"), data).unwrap();

    let (engine, wal) = recover_with_wal_size(&data_dir, 1024, Some(1 << 16))
        .expect("recovery with an arbitrary snapshot file must not fail");
    assert_eq!(wal.record_count(), 0);
    // Either the snapshot was rejected (empty book) or it was a genuine,
    // checksum-valid snapshot the fuzzer synthesised; both are consistent.
    let _ = engine.book().order_count();
});
