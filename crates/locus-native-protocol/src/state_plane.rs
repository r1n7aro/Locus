//! Byte layout of the broker state plane: a named shared-memory region the
//! broker writes its status JSON into, so clients read status without a pipe
//! round trip (and while the host's main thread is stuck).
//!
//! ```text
//! header (64 bytes, little endian)
//!   0  u32  magic "LCNB"            12 u32 writer process id
//!   4  u16  version                 16 u64 latest slot sequence (0 = none)
//!   6  u16  slot count              24 u64 latest observed-at (unix ms)
//!   8  u32  slot size
//! slots (slot count × slot size), written round-robin
//!   0  u64  slot sequence (0 while being written)
//!   8  u64  observed-at (unix ms)
//!  16  u32  source flags            20 u32 payload length
//!  24  ..   payload (UTF-8 JSON)
//! ```
//!
//! A reader takes the slot named by the header sequence and accepts it only
//! when the slot sequence matches before and after copying the payload and
//! the header sequence did not move meanwhile (no torn reads).

pub const MAGIC: u32 = 0x424e_434c; // "LCNB" little-endian.
pub const VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 64;
pub const SLOT_COUNT: usize = 8;
pub const SLOT_SIZE: usize = 128 * 1024;
pub const SLOT_PAYLOAD_OFFSET: usize = 24;
pub const MAX_PAYLOAD: usize = SLOT_SIZE - SLOT_PAYLOAD_OFFSET;
pub const SOURCE_BROKER: u32 = 0x1;

pub const fn total_size() -> usize {
    HEADER_SIZE + SLOT_COUNT * SLOT_SIZE
}

fn put(buf: &mut [u8], offset: usize, bytes: &[u8]) {
    if let Some(target) = buf.get_mut(offset..offset + bytes.len()) {
        target.copy_from_slice(bytes);
    }
}

fn get_u16(buf: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(buf.get(offset..offset + 2)?.try_into().ok()?))
}

fn get_u32(buf: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(buf.get(offset..offset + 4)?.try_into().ok()?))
}

fn get_u64(buf: &[u8], offset: usize) -> Option<u64> {
    Some(u64::from_le_bytes(buf.get(offset..offset + 8)?.try_into().ok()?))
}

/// Initialize the header of a freshly created plane.
pub fn write_header(buf: &mut [u8], process_id: u32, now_ms: u64) {
    put(buf, 0, &MAGIC.to_le_bytes());
    put(buf, 4, &VERSION.to_le_bytes());
    put(buf, 6, &(SLOT_COUNT.min(u16::MAX as usize) as u16).to_le_bytes());
    put(buf, 8, &(SLOT_SIZE.min(u32::MAX as usize) as u32).to_le_bytes());
    put(buf, 12, &process_id.to_le_bytes());
    put(buf, 16, &0u64.to_le_bytes());
    put(buf, 24, &now_ms.to_le_bytes());
}

/// Write `payload` as sequence `seq` (>= 1). Returns `false` and writes
/// nothing when the payload is empty or larger than [`MAX_PAYLOAD`].
pub fn write_slot(buf: &mut [u8], seq: u64, observed_at_ms: u64, source: u32, payload: &[u8]) -> bool {
    if seq == 0 || payload.is_empty() || payload.len() > MAX_PAYLOAD {
        return false;
    }
    let slot = ((seq - 1) as usize) % SLOT_COUNT;
    let offset = HEADER_SIZE + slot * SLOT_SIZE;
    if offset + SLOT_SIZE > buf.len() {
        return false;
    }
    put(buf, offset, &0u64.to_le_bytes());
    put(buf, offset + 8, &observed_at_ms.to_le_bytes());
    put(buf, offset + 16, &source.to_le_bytes());
    put(buf, offset + 20, &(payload.len().min(u32::MAX as usize) as u32).to_le_bytes());
    put(buf, offset + SLOT_PAYLOAD_OFFSET, payload);
    put(buf, offset, &seq.to_le_bytes());
    put(buf, 16, &seq.to_le_bytes());
    put(buf, 24, &observed_at_ms.to_le_bytes());
    true
}

/// One consistent payload read from the plane.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub seq: u64,
    pub observed_at_ms: u64,
    pub writer_process_id: u32,
    pub payload: Vec<u8>,
}

/// Read the latest complete payload, or `None` when the plane is empty,
/// foreign, or being rewritten right now (callers retry or fall back).
pub fn read_latest(buf: &[u8]) -> Option<Snapshot> {
    let magic = get_u32(buf, 0)?;
    let version = get_u16(buf, 4)?;
    let slot_count = get_u16(buf, 6)? as usize;
    let slot_size = get_u32(buf, 8)? as usize;
    let writer_process_id = get_u32(buf, 12)?;
    let writer_seq = get_u64(buf, 16)?;
    if magic != MAGIC
        || version != VERSION
        || slot_count == 0
        || slot_count > SLOT_COUNT
        || slot_size < 64
        || slot_size > SLOT_SIZE
        || writer_seq == 0
    {
        return None;
    }
    let slot_index = ((writer_seq - 1) as usize) % slot_count;
    let slot_offset = HEADER_SIZE.checked_add(slot_index.checked_mul(slot_size)?)?;
    let slot = buf.get(slot_offset..slot_offset.checked_add(slot_size)?)?;
    let seq_before = get_u64(slot, 0)?;
    if seq_before != writer_seq {
        return None;
    }
    let observed_at_ms = get_u64(slot, 8)?;
    let payload_len = get_u32(slot, 20)? as usize;
    if payload_len == 0 || payload_len > slot_size.saturating_sub(SLOT_PAYLOAD_OFFSET) {
        return None;
    }
    let payload = slot
        .get(SLOT_PAYLOAD_OFFSET..SLOT_PAYLOAD_OFFSET + payload_len)?
        .to_vec();
    if get_u64(slot, 0)? != seq_before || get_u64(buf, 16)? != writer_seq {
        return None;
    }
    Some(Snapshot {
        seq: writer_seq,
        observed_at_ms,
        writer_process_id,
        payload,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layout_is_pinned() {
        // Brokers and clients of every version read this exact layout.
        assert_eq!(MAGIC.to_le_bytes(), *b"LCNB");
        assert_eq!(total_size(), 64 + 8 * 128 * 1024);
        let mut buf = vec![0u8; total_size()];
        write_header(&mut buf, 0x01020304, 99);
        assert_eq!(&buf[0..16], &[b'L', b'C', b'N', b'B', 1, 0, 8, 0, 0, 0, 2, 0, 4, 3, 2, 1]);
        assert!(write_slot(&mut buf, 1, 1234, SOURCE_BROKER, br#"{"a":1}"#));
        let slot = &buf[HEADER_SIZE..HEADER_SIZE + 31];
        assert_eq!(&slot[0..8], &1u64.to_le_bytes());
        assert_eq!(&slot[8..16], &1234u64.to_le_bytes());
        assert_eq!(&slot[16..20], &1u32.to_le_bytes());
        assert_eq!(&slot[20..24], &7u32.to_le_bytes());
        assert_eq!(&slot[24..31], br#"{"a":1}"#);
        assert_eq!(&buf[16..24], &1u64.to_le_bytes());
    }

    #[test]
    fn reader_returns_latest_slot_and_rejects_torn_or_foreign_planes() {
        let mut buf = vec![0u8; total_size()];
        assert!(read_latest(&buf).is_none());
        write_header(&mut buf, 7, 0);
        assert!(read_latest(&buf).is_none());
        for seq in 1..=11u64 {
            let payload = format!(r#"{{"seq":{seq}}}"#);
            assert!(write_slot(&mut buf, seq, seq * 10, SOURCE_BROKER, payload.as_bytes()));
        }
        let snapshot = read_latest(&buf).unwrap();
        assert_eq!(snapshot.seq, 11);
        assert_eq!(snapshot.observed_at_ms, 110);
        assert_eq!(snapshot.writer_process_id, 7);
        assert_eq!(snapshot.payload, br#"{"seq":11}"#);

        // A slot being rewritten (sequence zeroed) is rejected.
        let slot = HEADER_SIZE + ((11 - 1) % SLOT_COUNT) * SLOT_SIZE;
        buf[slot..slot + 8].copy_from_slice(&0u64.to_le_bytes());
        assert!(read_latest(&buf).is_none());

        assert!(!write_slot(&mut buf, 12, 0, SOURCE_BROKER, &vec![b'x'; MAX_PAYLOAD + 1]));
        assert!(!write_slot(&mut buf, 0, 0, SOURCE_BROKER, b"{}"));
        buf[0] = b'X';
        assert!(read_latest(&buf).is_none());
    }
}
