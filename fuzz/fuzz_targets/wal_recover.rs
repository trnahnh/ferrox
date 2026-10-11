//! Arbitrary bytes as a WAL file. Recovery must never panic, and the only
//! error it may return is a protocol error for a record whose CRC is valid
//! but whose payload is not a message: that is a corrupted-but-CRC-colliding
//! or foreign log, and the engine refuses to start on it rather than guess.
//! Everything else (torn record, bad CRC, trailing garbage) is a valid
//! empty-or-prefix log. Both outcomes must be deterministic: recovering the
//! recovered log again gives the same count and state, or the same error.
//!
//! The first 60-second CI run of this target (2026-10-11, GitHub Actions,
//! 87,568 executions) found the one-byte record `0xFF`: its CRC32 is
//! `0xFF000000`, so the WAL accepts it and the decoder rejects it with
//! `UnknownMessageType(255)`. The original oracle said recovery must always
//! succeed; that was the oracle's mistake, and the input is now a seed in
//! `fuzz/corpus/wal_recover/` plus a unit test in `src/recovery.rs`.
#![no_main]
use ferrox::recovery::{RecoveryError, recover_with_wal_size};
use ferrox::wal::WalError;
use libfuzzer_sys::fuzz_target;

fuzz_target!(|data: &[u8]| {
    let dir = tempfile::tempdir().unwrap();
    let data_dir = dir.path().join("data");
    std::fs::create_dir_all(&data_dir).unwrap();
    std::fs::write(data_dir.join("wal.bin"), data).unwrap();

    match recover_with_wal_size(&data_dir, 1024, Some(1 << 16)) {
        Ok((engine, wal)) => {
            let count = wal.record_count();
            // Every decoded record carries a message of at least 16 bytes, so
            // records are at least 24 bytes and this bounds the count.
            assert!(count as usize <= data.len() / 24 + 1);
            assert!(wal.write_pos() as usize <= data.len().max(8));
            let state = engine.book().all_resting_orders();
            drop((engine, wal));

            let (engine2, wal2) = recover_with_wal_size(&data_dir, 1024, Some(1 << 16))
                .expect("second recovery of a recovered log must succeed");
            assert_eq!(wal2.record_count(), count);
            assert_eq!(engine2.book().all_resting_orders(), state);
        }
        Err(RecoveryError::Wal(WalError::Protocol(first))) => {
            // CRC-valid, undecodable record: refusing to start is the policy.
            // It must be the same refusal every time.
            match recover_with_wal_size(&data_dir, 1024, Some(1 << 16)) {
                Err(RecoveryError::Wal(WalError::Protocol(second))) => assert_eq!(first, second),
                Ok(_) => panic!("recovery failed with {first:?} and then succeeded"),
                Err(e) => panic!("recovery failed with {first:?} and then with {e:?}"),
            }
        }
        Err(e) => panic!("recovery of an arbitrary log failed with a non-protocol error: {e:?}"),
    }
});
