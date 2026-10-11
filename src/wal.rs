use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use memmap2::MmapMut;

use crate::protocol::{self, EngineCommand, NEW_ORDER_SIZE};

/// WAL record header size: 4 bytes payload_len + 4 bytes CRC32.
const HEADER_SIZE: usize = 8;

const ALIGNMENT: usize = 8;

const DEFAULT_INITIAL_SIZE: u64 = 64 * 1024 * 1024;

fn align_up(n: usize) -> usize {
    (n + ALIGNMENT - 1) & !(ALIGNMENT - 1)
}

#[derive(Debug)]
pub enum WalError {
    Io(io::Error),
    Protocol(protocol::ProtocolError),
    Corruption { offset: u64 },
    TruncatedRecord { offset: u64 },
}

impl std::fmt::Display for WalError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Io(e) => write!(f, "wal io error: {e}"),
            Self::Protocol(e) => write!(f, "wal protocol error: {e}"),
            Self::Corruption { offset } => write!(f, "wal corruption at offset {offset}"),
            Self::TruncatedRecord { offset } => {
                write!(f, "wal truncated record at offset {offset}")
            }
        }
    }
}

impl std::error::Error for WalError {}

impl From<io::Error> for WalError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<protocol::ProtocolError> for WalError {
    fn from(e: protocol::ProtocolError) -> Self {
        Self::Protocol(e)
    }
}

/// Append-only write-ahead log backed by a memory-mapped file.
///
/// Record format on disk:
/// ```text
/// [payload_len: u32 LE][crc32: u32 LE][payload: N bytes][padding to 8-byte align]
/// ```
pub struct Wal {
    mmap: MmapMut,
    file: File,
    #[allow(dead_code)]
    path: PathBuf,
    write_pos: u64,
    mapped_size: u64,
    encode_buf: [u8; NEW_ORDER_SIZE], // pre-allocated, max payload size
    record_count: u64,
}

impl Wal {
    /// Open or create a WAL file. On reopen, scans existing records to restore
    /// `write_pos` and `record_count`.
    pub fn open(path: impl AsRef<Path>) -> Result<Self, WalError> {
        Self::open_with_size(path, DEFAULT_INITIAL_SIZE)
    }

    /// Open with a custom initial mmap size (useful for tests).
    pub fn open_with_size(path: impl AsRef<Path>, initial_size: u64) -> Result<Self, WalError> {
        let path = path.as_ref().to_path_buf();

        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(false)
            .open(&path)?;

        let file_len = file.metadata()?.len();
        let mapped_size = if file_len < initial_size {
            file.set_len(initial_size)?;
            initial_size
        } else {
            file_len
        };

        // SAFETY: Single-writer invariant — only the matching thread accesses
        // this file. No other process reads/writes it concurrently.
        let mmap = unsafe { MmapMut::map_mut(&file)? };

        let scan = scan_records(&mmap);

        // Establish the module invariant: every byte at or beyond `write_pos`
        // is zero. Whatever follows the last valid record (a torn record, a
        // corrupt one, or a zeroed header) may still be followed by *older,
        // valid records* left over from before a previous crash. If they were
        // left in place, new appends that happen to realign with the old
        // record boundaries would make them readable again on the next
        // recovery (found by tests/crash_recovery.rs). Discarding them at the
        // file level is O(1) and the re-extended region reads as zeros, so no
        // page up to `mapped_size` has to be dirtied.
        drop(mmap);
        file.set_len(scan.end)?;
        file.set_len(mapped_size)?;
        // SAFETY: as above; the previous mapping has been dropped.
        let mmap = unsafe { MmapMut::map_mut(&file)? };

        Ok(Self {
            mmap,
            file,
            path,
            write_pos: scan.end,
            mapped_size,
            encode_buf: [0u8; NEW_ORDER_SIZE],
            record_count: scan.count,
        })
    }

    /// Append an `EngineCommand` to the WAL. Returns the record number (1-based).
    pub fn append(&mut self, cmd: &EngineCommand) -> Result<u64, WalError> {
        let payload_len = match cmd {
            EngineCommand::NewOrder(order) => {
                protocol::encode_new_order(&mut self.encode_buf, order)?
            }
            EngineCommand::CancelOrder { order_id } => {
                protocol::encode_cancel_order(&mut self.encode_buf, *order_id)?
            }
        };

        let record_size = align_up(HEADER_SIZE + payload_len);
        self.ensure_capacity(record_size as u64)?;

        let pos = self.write_pos as usize;

        let crc = crc32fast::hash(&self.encode_buf[..payload_len]);
        self.mmap[pos..pos + 4].copy_from_slice(&(payload_len as u32).to_le_bytes());
        self.mmap[pos + 4..pos + 8].copy_from_slice(&crc.to_le_bytes());

        self.mmap[pos + HEADER_SIZE..pos + HEADER_SIZE + payload_len]
            .copy_from_slice(&self.encode_buf[..payload_len]);

        let pad_start = pos + HEADER_SIZE + payload_len;
        let pad_end = pos + record_size;
        if pad_end > pad_start {
            self.mmap[pad_start..pad_end].fill(0);
        }

        self.write_pos += record_size as u64;
        self.record_count += 1;

        Ok(self.record_count)
    }

    pub fn record_count(&self) -> u64 {
        self.record_count
    }

    /// Byte offset of the next append, i.e. the length of the valid log.
    pub fn write_pos(&self) -> u64 {
        self.write_pos
    }

    /// Iterate all valid records starting from record number `start_record` (1-based).
    /// Pass 0 to iterate from the very beginning.
    pub fn iter_from(&self, start_record: u64) -> WalIterator<'_> {
        WalIterator {
            mmap: &self.mmap,
            read_pos: 0,
            end_pos: self.write_pos,
            current_record: 0,
            start_record,
        }
    }

    pub fn truncate_to(&mut self, offset: u64, record_count: u64) -> Result<(), WalError> {
        let start = offset as usize;
        let end = self.write_pos as usize;
        if end > start {
            self.mmap[start..end].fill(0);
        }
        self.write_pos = offset;
        self.record_count = record_count;
        Ok(())
    }

    pub fn flush_async(&self) -> Result<(), WalError> {
        self.mmap.flush_async().map_err(WalError::Io)
    }

    fn ensure_capacity(&mut self, needed: u64) -> Result<(), WalError> {
        if self.write_pos + needed <= self.mapped_size {
            return Ok(());
        }

        let new_size = (self.mapped_size * 2).max(self.write_pos + needed);
        self.file.set_len(new_size)?;

        // SAFETY: Same single-writer invariant as open.
        self.mmap = unsafe { MmapMut::map_mut(&self.file)? };
        self.mapped_size = new_size;

        Ok(())
    }
}

/// Result of walking the records at the start of a mapped log.
struct Scan {
    /// Byte offset just past the last valid record.
    end: u64,
    /// Number of valid records.
    count: u64,
}

/// Walks records from offset 0 until the first zero header, torn record,
/// CRC mismatch, or end of data. Nothing at or after `end` is trusted.
fn scan_records(data: &[u8]) -> Scan {
    let file_len = data.len() as u64;
    let mut pos: u64 = 0;
    let mut count: u64 = 0;

    loop {
        if pos + HEADER_SIZE as u64 > file_len {
            break;
        }

        let p = pos as usize;
        let payload_len = u32::from_le_bytes(data[p..p + 4].try_into().unwrap()) as usize;

        // A zero payload_len means unwritten space (or a header whose first
        // byte was lost; either way nothing after it can be trusted).
        if payload_len == 0 {
            break;
        }

        let record_size = align_up(HEADER_SIZE + payload_len);
        if pos + record_size as u64 > file_len {
            // Truncated record — discard it.
            break;
        }

        let stored_crc = u32::from_le_bytes(data[p + 4..p + 8].try_into().unwrap());
        let computed_crc = crc32fast::hash(&data[p + HEADER_SIZE..p + HEADER_SIZE + payload_len]);

        if stored_crc != computed_crc {
            break;
        }

        pos += record_size as u64;
        count += 1;
    }

    Scan { end: pos, count }
}

pub struct WalIterator<'a> {
    mmap: &'a [u8],
    read_pos: u64,
    end_pos: u64,
    current_record: u64,
    start_record: u64,
}

impl<'a> Iterator for WalIterator<'a> {
    type Item = Result<(u64, EngineCommand), WalError>;

    fn next(&mut self) -> Option<Self::Item> {
        loop {
            if self.read_pos + HEADER_SIZE as u64 > self.end_pos {
                return None;
            }

            let p = self.read_pos as usize;
            let payload_len = u32::from_le_bytes(self.mmap[p..p + 4].try_into().unwrap()) as usize;

            if payload_len == 0 {
                return None;
            }

            let record_size = align_up(HEADER_SIZE + payload_len);
            if self.read_pos + record_size as u64 > self.end_pos {
                return Some(Err(WalError::TruncatedRecord {
                    offset: self.read_pos,
                }));
            }

            let stored_crc = u32::from_le_bytes(self.mmap[p + 4..p + 8].try_into().unwrap());
            let payload = &self.mmap[p + HEADER_SIZE..p + HEADER_SIZE + payload_len];
            let computed_crc = crc32fast::hash(payload);

            if stored_crc != computed_crc {
                return Some(Err(WalError::Corruption {
                    offset: self.read_pos,
                }));
            }

            self.read_pos += record_size as u64;
            self.current_record += 1;

            // Skip records before start_record
            if self.current_record <= self.start_record {
                continue;
            }

            match protocol::decode_message(payload) {
                Ok(cmd) => return Some(Ok((self.current_record, cmd))),
                Err(e) => return Some(Err(WalError::Protocol(e))),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::order::{Order, Side};

    fn make_order(id: u64) -> Order {
        Order {
            id,
            trader_id: 1,
            side: Side::Bid,
            price: 15005,
            quantity: 100,
            timestamp: 1_000_000,
        }
    }

    fn new_order_cmd(id: u64) -> EngineCommand {
        EngineCommand::NewOrder(make_order(id))
    }

    fn cancel_cmd(id: u64) -> EngineCommand {
        EngineCommand::CancelOrder { order_id: id }
    }

    #[test]
    fn create_new_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let wal = Wal::open(&path).unwrap();
        assert_eq!(wal.record_count(), 0);
        assert_eq!(wal.write_pos(), 0);
        assert!(path.exists());
    }

    #[test]
    fn append_single_new_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        let seq = wal.append(&new_order_cmd(1)).unwrap();

        assert_eq!(seq, 1);
        assert_eq!(wal.record_count(), 1);
        // NewOrder payload = 40 bytes, record = align_up(8 + 40) = 48 bytes
        assert_eq!(wal.write_pos(), 48);
    }

    #[test]
    fn append_cancel_order() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        let seq = wal.append(&cancel_cmd(42)).unwrap();

        assert_eq!(seq, 1);
        // CancelOrder payload = 16 bytes, record = align_up(8 + 16) = 24 bytes
        assert_eq!(wal.write_pos(), 24);
    }

    #[test]
    fn append_multiple_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        for i in 1..=100 {
            let seq = wal.append(&new_order_cmd(i)).unwrap();
            assert_eq!(seq, i);
        }

        assert_eq!(wal.record_count(), 100);
        assert_eq!(wal.write_pos(), 100 * 48);
    }

    #[test]
    fn iterate_all_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        wal.append(&new_order_cmd(10)).unwrap();
        wal.append(&cancel_cmd(10)).unwrap();
        wal.append(&new_order_cmd(20)).unwrap();

        let records: Vec<_> = wal.iter_from(0).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(records.len(), 3);

        assert_eq!(records[0].0, 1); // record number
        match &records[0].1 {
            EngineCommand::NewOrder(o) => assert_eq!(o.id, 10),
            _ => panic!("expected NewOrder"),
        }

        assert_eq!(records[1].0, 2);
        assert_eq!(records[1].1, EngineCommand::CancelOrder { order_id: 10 });

        assert_eq!(records[2].0, 3);
        match &records[2].1 {
            EngineCommand::NewOrder(o) => assert_eq!(o.id, 20),
            _ => panic!("expected NewOrder"),
        }
    }

    #[test]
    fn iterate_from_offset() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        for i in 1..=10 {
            wal.append(&new_order_cmd(i)).unwrap();
        }

        // Start from record 5 — should yield records 6..=10
        let records: Vec<_> = wal.iter_from(5).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(records.len(), 5);
        assert_eq!(records[0].0, 6);
        assert_eq!(records[4].0, 10);
    }

    #[test]
    fn iterate_empty_wal() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let wal = Wal::open(&path).unwrap();
        let records: Vec<_> = wal.iter_from(0).collect::<Result<Vec<_>, _>>().unwrap();
        assert!(records.is_empty());
    }

    #[test]
    fn reopen_preserves_state() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        {
            let mut wal = Wal::open(&path).unwrap();
            wal.append(&new_order_cmd(1)).unwrap();
            wal.append(&new_order_cmd(2)).unwrap();
            wal.append(&cancel_cmd(1)).unwrap();
        }

        let wal = Wal::open(&path).unwrap();
        assert_eq!(wal.record_count(), 3);
        assert_eq!(wal.write_pos(), 48 + 48 + 24); // two NewOrders + one Cancel

        let records: Vec<_> = wal.iter_from(0).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(records.len(), 3);
    }

    #[test]
    fn record_format_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        wal.append(&new_order_cmd(42)).unwrap();

        let payload_len = u32::from_le_bytes(wal.mmap[0..4].try_into().unwrap());
        assert_eq!(payload_len, NEW_ORDER_SIZE as u32);

        let stored_crc = u32::from_le_bytes(wal.mmap[4..8].try_into().unwrap());
        let computed_crc = crc32fast::hash(&wal.mmap[8..8 + NEW_ORDER_SIZE]);
        assert_eq!(stored_crc, computed_crc);

        // First byte of payload is the message type
        assert_eq!(wal.mmap[8], protocol::MSG_NEW_ORDER);
    }

    #[test]
    fn corrupt_crc_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        wal.append(&new_order_cmd(1)).unwrap();
        wal.append(&new_order_cmd(2)).unwrap();

        // Corrupt the CRC of the second record (at offset 48)
        wal.mmap[48 + 4] ^= 0xFF;

        // Iterator should yield first record, then error on second
        let mut iter = wal.iter_from(0);
        assert!(iter.next().unwrap().is_ok());
        let err = iter.next().unwrap().unwrap_err();
        matches!(err, WalError::Corruption { offset: 48 });
    }

    #[test]
    fn corrupt_payload_detected() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        wal.append(&new_order_cmd(1)).unwrap();

        // Corrupt a payload byte
        wal.mmap[HEADER_SIZE + 5] ^= 0xFF;

        let mut iter = wal.iter_from(0);
        let err = iter.next().unwrap().unwrap_err();
        matches!(err, WalError::Corruption { offset: 0 });
    }

    #[test]
    fn reopen_detects_truncated_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        {
            let mut wal = Wal::open(&path).unwrap();
            wal.append(&new_order_cmd(1)).unwrap();
            wal.append(&new_order_cmd(2)).unwrap();

            // Simulate a crash: write a partial header for a third record
            let pos = wal.write_pos() as usize;
            wal.mmap[pos..pos + 4].copy_from_slice(&(40u32).to_le_bytes());
            // CRC and payload not written — truncated
        }

        // Reopen should find only 2 valid records
        let wal = Wal::open(&path).unwrap();
        assert_eq!(wal.record_count(), 2);
    }

    #[test]
    fn reopen_detects_corrupt_record() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        {
            let mut wal = Wal::open(&path).unwrap();
            wal.append(&new_order_cmd(1)).unwrap();
            wal.append(&new_order_cmd(2)).unwrap();
            wal.append(&new_order_cmd(3)).unwrap();

            // Corrupt record 2's CRC
            wal.mmap[48 + 4] ^= 0xFF;
        }

        // Reopen should find only 1 valid record (stops at corruption)
        let wal = Wal::open(&path).unwrap();
        assert_eq!(wal.record_count(), 1);
        assert_eq!(wal.write_pos(), 48);
    }

    #[test]
    fn truncate_to_discards_records() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        wal.append(&new_order_cmd(1)).unwrap();
        wal.append(&new_order_cmd(2)).unwrap();
        wal.append(&new_order_cmd(3)).unwrap();

        // Truncate to after the first record
        wal.truncate_to(48, 1).unwrap();
        assert_eq!(wal.record_count(), 1);
        assert_eq!(wal.write_pos(), 48);

        let records: Vec<_> = wal.iter_from(0).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(records.len(), 1);
    }

    #[test]
    fn remap_on_growth() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        // Start with a tiny mmap (256 bytes — room for 5 NewOrder records)
        let mut wal = Wal::open_with_size(&path, 256).unwrap();
        assert_eq!(wal.mapped_size, 256);

        // Write 10 records — should trigger remap
        for i in 1..=10 {
            wal.append(&new_order_cmd(i)).unwrap();
        }

        assert!(wal.mapped_size > 256);
        assert_eq!(wal.record_count(), 10);

        let records: Vec<_> = wal.iter_from(0).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(records.len(), 10);
    }

    #[test]
    fn mixed_new_order_and_cancel() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        wal.append(&new_order_cmd(1)).unwrap();
        wal.append(&new_order_cmd(2)).unwrap();
        wal.append(&cancel_cmd(1)).unwrap();
        wal.append(&new_order_cmd(3)).unwrap();

        let records: Vec<_> = wal.iter_from(0).collect::<Result<Vec<_>, _>>().unwrap();
        assert_eq!(records.len(), 4);

        assert!(matches!(records[0].1, EngineCommand::NewOrder(_)));
        assert!(matches!(records[1].1, EngineCommand::NewOrder(_)));
        assert!(matches!(records[2].1, EngineCommand::CancelOrder { .. }));
        assert!(matches!(records[3].1, EngineCommand::NewOrder(_)));

        // Verify write positions: 48 + 48 + 24 + 48 = 168
        assert_eq!(wal.write_pos(), 168);
    }

    #[test]
    fn flush_async_does_not_error() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let mut wal = Wal::open(&path).unwrap();
        wal.append(&new_order_cmd(1)).unwrap();
        wal.flush_async().unwrap();
    }

    #[test]
    fn new_order_preserves_fields() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("wal.bin");

        let order = Order {
            id: 999,
            trader_id: 42,
            side: Side::Ask,
            price: -12345,
            quantity: u64::MAX,
            timestamp: 0, // timestamp not encoded in protocol
        };

        let mut wal = Wal::open(&path).unwrap();
        wal.append(&EngineCommand::NewOrder(order)).unwrap();

        let records: Vec<_> = wal.iter_from(0).collect::<Result<Vec<_>, _>>().unwrap();
        match &records[0].1 {
            EngineCommand::NewOrder(o) => {
                assert_eq!(o.id, 999);
                assert_eq!(o.trader_id, 42);
                assert_eq!(o.side, Side::Ask);
                assert_eq!(o.price, -12345);
                assert_eq!(o.quantity, u64::MAX);
            }
            _ => panic!("expected NewOrder"),
        }
    }
}
