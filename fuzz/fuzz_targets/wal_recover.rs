//! Arbitrary bytes as a WAL file: recovery must never panic, must always
//! succeed (a damaged log is a valid empty-or-prefix log, not an error), and
//! must be idempotent (recovering the recovered log gives the same count).
#![no_main]
use ferrox::recovery::recover_with_wal_size;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("wal.bin"), data).unwrap();

    let (engine, wal) = recover_with_wal_size(&data_dir, 1024, Some(1 << 16))
        .expect("recovery of an arbitrary log must not fail");
    let count = wal.record_count();
    // 24 bytes is the smallest record, so this bounds the count.
    assert!(count as usize <= data.len() / 24 + 1);
    assert!(wal.write_pos() as usize <= data.len().max(8));
    let state = engine.book().all_resting_orders();
    drop((engine, wal));

    let (engine2, wal2) = recover_with_wal_size(&data_dir, 1024, Some(1 << 16)).unwrap();
    assert_eq!(wal2.record_count(), count);
    assert_eq!(engine2.book().all_resting_orders(), state);
});
