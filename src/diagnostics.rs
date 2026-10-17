//! Fixed-size, best-effort journal in host-managed storage. No raw provider data.
//! Hot append reads metadata + one chunk; bounded scans occur on recovery, daily
//! age cleanup and read-only export. Intent-before-data detects interrupted writes.
use serde::{Deserialize, Serialize};
use std::fmt;

pub const PREFIX: &str = "diag/v1/";
const META: &str = "diag/v1/meta";
const CHUNK_SLOTS: usize = 64;
const RECORDS_PER_CHUNK: usize = 24;
pub const MAX_RECORDS: usize = CHUNK_SLOTS * RECORDS_PER_CHUNK;
/// Static upper bound including all slot keys, chunk bytes and metadata. This
/// leaves 496 KiB of the shared MiB, NOT a promise that unlimited mat state fits.
/// Essential writes may reclaim the journal; see `reclaim`.
pub const MAX_BYTES: usize = 528 * 1024;
pub const MAX_RECORD_BYTES: usize = 320;
pub const MAX_AGE: i64 = 30 * 24 * 60 * 60;
const MAX_CHUNK_BYTES: usize = 8 * 1024;
const MAX_META_BYTES: usize = 1024;
const _: () = assert!(CHUNK_SLOTS * (MAX_CHUNK_BYTES + 32) + MAX_META_BYTES + 32 <= MAX_BYTES);

pub trait Backend {
    type Error: fmt::Display;
    fn get(&mut self, key: &str) -> Result<Option<String>, Self::Error>;
    fn set(&mut self, key: &str, value: &str) -> Result<(), Self::Error>;
    fn delete(&mut self, key: &str) -> Result<(), Self::Error>;
    fn list(&mut self, prefix: &str) -> Result<Vec<String>, Self::Error>;
}
#[derive(Clone, Debug, Deserialize, Serialize, PartialEq, Eq)]
#[serde(deny_unknown_fields)]
pub struct Record {
    pub received_at: i64,
    pub provider_at: Option<i64>,
    pub mat: Option<String>,
    pub event: String,
    pub stage: String,
    pub outcome: String,
    pub correlation: u64,
    pub count: u32,
    pub first_at: i64,
    pub last_at: i64,
}
#[derive(Clone, Debug, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Meta {
    next: u64,
    correlation: u64,
    slots: u64,
    pending: bool,
    pruned_at: i64,
    truncated: u64,
    suppressed: u64,
    write_failures: u64,
    gaps: u64,
}
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Export {
    pub records: Vec<Record>,
    pub oldest: Option<i64>,
    pub newest: Option<i64>,
    pub used_bytes: usize,
    pub truncation_count: u64,
    pub suppression_count: u64,
    pub write_failure_count: u64,
    pub gap_count: u64,
    /// False on known loss. This never proves delivery of an absent provider event.
    pub complete: bool,
    pub counters_exact: bool,
    pub total: usize,
    pub page: usize,
    pub per_page: usize,
    pub snapshot: String,
    pub has_more: bool,
    pub expired_records: usize,
}
#[derive(Debug, PartialEq, Eq)]
pub enum Error {
    Storage,
    Corrupt,
    Invalid,
    SnapshotChanged,
}
#[derive(Clone, Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct Chunk {
    sequence: u64,
    end: usize,
    records: Vec<Record>,
}
fn key(slot: usize) -> String {
    format!("{PREFIX}chunk/{slot:02}")
}
fn encoded<T: Serialize>(value: &T) -> String {
    serde_json::to_string(value).expect("bounded journal serialization")
}
fn hash(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf29ce484222325, |h, b| {
        (h ^ u64::from(*b)).wrapping_mul(0x100000001b3)
    })
}
pub fn safe_mat_ref(id: &str) -> Option<String> {
    if id.is_empty() || id.len() > 384 || id.bytes().any(|b| b.is_ascii_control()) {
        return None;
    }
    Some(format!("mat_{:016x}", hash(id.as_bytes())))
}
fn valid_record(r: &Record) -> bool {
    let scoped = r.mat.as_ref().is_some_and(|m| {
        m.len() == 20
            && m.starts_with("mat_")
            && m.as_bytes()[4..].iter().all(u8::is_ascii_hexdigit)
    }) && r.provider_at.is_some();
    let unscoped = r.mat.is_none() && r.provider_at.is_none();
    let schema = match (r.event.as_str(), r.stage.as_str(), r.outcome.as_str()) {
        ("bed_in" | "bed_out", "receipt", "received") => scoped && r.count == 1,
        (
            "bed_in" | "bed_out",
            "decision",
            "transition" | "unchanged" | "duplicate_or_stale" | "conflict",
        ) => scoped && r.count == 1,
        ("bed_in" | "bed_out", "persistence", "failed") => scoped && r.count == 1,
        (
            "bed_in" | "bed_out",
            "publication",
            "reading_failed" | "action_failed" | "action_published" | "no_action",
        ) => scoped && r.count == 1,
        ("webhook", "receipt", "received" | "suppressed")
        | ("webhook", "decision", "blocked_or_rejected") => unscoped,
        (
            "lifecycle",
            "startup" | "reconcile",
            "ready"
            | "blocked"
            | "disconnected"
            | "refresh_store_failed"
            | "refresh_failed"
            | "authorization_failed"
            | "subscription_failed"
            | "discovery_failed",
        ) => unscoped,
        ("callback", "receipt", "received")
        | ("callback", "decision", "denied" | "failed")
        | ("callback", "activation", "complete") => unscoped,
        _ => false,
    };
    schema
        && r.count > 0
        && r.first_at > 0
        && r.first_at <= r.received_at
        && r.received_at <= r.last_at
        && r.provider_at.is_none_or(|v| v > 0)
        && encoded(r).len() <= MAX_RECORD_BYTES
}
fn decode_chunk(raw: &str, slot: usize) -> Result<Chunk, Error> {
    if raw.len() > MAX_CHUNK_BYTES {
        return Err(Error::Corrupt);
    }
    let c: Chunk = serde_json::from_str(raw).map_err(|_| Error::Corrupt)?;
    if c.sequence % CHUNK_SLOTS as u64 != slot as u64
        || c.records.is_empty()
        || c.records.len() > c.end
        || c.end > RECORDS_PER_CHUNK
        || c.records.iter().any(|r| !valid_record(r))
    {
        return Err(Error::Corrupt);
    }
    Ok(c)
}
struct Scan {
    chunks: Vec<(usize, Chunk)>,
    bytes: usize,
    slots: u64,
    gaps: u64,
}
fn scan<B: Backend>(b: &mut B) -> Result<Scan, Error> {
    let mut s = Scan {
        chunks: vec![],
        bytes: 0,
        slots: 0,
        gaps: 0,
    };
    for slot in 0..CHUNK_SLOTS {
        if let Some(raw) = b.get(&key(slot)).map_err(|_| Error::Storage)? {
            s.bytes += key(slot).len() + raw.len();
            s.slots |= 1 << slot;
            match decode_chunk(&raw, slot) {
                Ok(c) => s.chunks.push((slot, c)),
                Err(_) => s.gaps += 1,
            }
        }
    }
    s.chunks.sort_by_key(|(_, c)| c.sequence);
    Ok(s)
}
fn raw_meta<B: Backend>(b: &mut B) -> Result<Option<String>, Error> {
    b.get(META).map_err(|_| Error::Storage)
}
fn decode_meta(raw: &str) -> Option<Meta> {
    if raw.len() > MAX_META_BYTES {
        return None;
    }
    serde_json::from_str::<Meta>(raw)
        .ok()
        .filter(|m| m.pruned_at >= 0)
}
/// Recovery is read-only; the next mutating operation durably saves the recovered
/// cursor and gap. Counters lost with metadata cannot be reconstructed exactly.
fn recover(raw: Option<&str>, s: &Scan) -> Meta {
    let decoded = raw.and_then(decode_meta);
    let missing = decoded.is_none();
    let mut m = decoded.unwrap_or_default();
    m.gaps = m
        .gaps
        .saturating_add(s.gaps)
        .saturating_add((m.slots & !s.slots).count_ones() as u64);
    if m.pending || (missing && (raw.is_some() || s.slots != 0)) {
        m.gaps = m.gaps.saturating_add(1);
    }
    for (_, c) in &s.chunks {
        m.next = m.next.max(
            c.sequence
                .saturating_mul(RECORDS_PER_CHUNK as u64)
                .saturating_add(c.end as u64),
        );
        m.correlation = m
            .correlation
            .max(c.records.iter().map(|r| r.correlation).max().unwrap_or(0));
    }
    m.slots = s.slots;
    m.pending = false;
    m
}
fn load_meta<B: Backend>(b: &mut B) -> Result<Meta, Error> {
    let raw = raw_meta(b)?;
    if let Some(m) = raw.as_deref().and_then(decode_meta).filter(|m| !m.pending) {
        return Ok(m);
    }
    Ok(recover(raw.as_deref(), &scan(b)?))
}
fn save_meta<B: Backend>(b: &mut B, m: &Meta) -> Result<(), Error> {
    let raw = encoded(m);
    if raw.len() > MAX_META_BYTES {
        return Err(Error::Invalid);
    }
    b.set(META, &raw).map_err(|_| Error::Storage)
}
fn prune<B: Backend>(b: &mut B, m: &mut Meta, at: i64) -> Result<(), Error> {
    if m.pruned_at != 0 && at >= m.pruned_at && at - m.pruned_at < 86400 {
        return Ok(());
    }
    let s = scan(b)?;
    m.gaps = m
        .gaps
        .saturating_add(s.gaps)
        .saturating_add((m.slots & !s.slots).count_ones() as u64);
    m.slots = s.slots;
    m.pending = true;
    save_meta(b, m)?;
    for (slot, mut c) in s.chunks {
        let before = c.records.len();
        c.records
            .retain(|r| r.last_at >= at.saturating_sub(MAX_AGE));
        if c.records.len() != before {
            if c.records.is_empty() {
                b.delete(&key(slot)).map_err(|_| Error::Storage)?;
                m.slots &= !(1 << slot);
            } else {
                b.set(&key(slot), &encoded(&c))
                    .map_err(|_| Error::Storage)?;
            }
            m.truncated = m
                .truncated
                .saturating_add((before - c.records.len()) as u64);
        }
    }
    m.pruned_at = at;
    m.pending = false;
    save_meta(b, m)
}
/// No implicit merging of event attempts. Noise callers submit explicit bounded
/// count/first/last summaries; accepted events always remain individual records.
pub fn append<B: Backend>(b: &mut B, r: Record) -> Result<(), Error> {
    if !valid_record(&r) {
        return Err(Error::Invalid);
    }
    let mut m = load_meta(b)?;
    prune(b, &mut m, r.received_at)?;
    let sequence = m.next / RECORDS_PER_CHUNK as u64;
    let slot = (sequence % CHUNK_SLOTS as u64) as usize;
    let raw = b.get(&key(slot)).map_err(|_| Error::Storage)?;
    let existing = raw.as_deref().map(|s| decode_chunk(s, slot));
    let mut target = Chunk {
        sequence,
        end: 0,
        records: vec![],
    };
    match existing {
        Some(Ok(c))
            if c.sequence == sequence && c.end == (m.next % RECORDS_PER_CHUNK as u64) as usize =>
        {
            target = c
        }
        Some(Ok(c)) if c.sequence != sequence => {
            m.truncated = m.truncated.saturating_add(c.records.len() as u64)
        }
        Some(Ok(c)) => {
            m.gaps = m.gaps.saturating_add(1);
            m.next = c
                .sequence
                .checked_mul(RECORDS_PER_CHUNK as u64)
                .and_then(|n| n.checked_add(c.end as u64))
                .ok_or(Error::Invalid)?;
            save_meta(b, &m)?;
            return append(b, r);
        }
        Some(Err(_)) => {
            m.gaps = m.gaps.saturating_add(1);
            m.pending = true;
            save_meta(b, &m)?;
            b.delete(&key(slot)).map_err(|_| Error::Storage)?;
            m.slots &= !(1 << slot);
            m.next = (sequence.checked_add(1).ok_or(Error::Invalid)?)
                .checked_mul(RECORDS_PER_CHUNK as u64)
                .ok_or(Error::Invalid)?;
            m.pending = false;
            save_meta(b, &m)?;
            return Err(Error::Corrupt);
        }
        None if m.next % RECORDS_PER_CHUNK as u64 != 0 => {
            // A current chunk can have expired completely. Advance to the next
            // sequence; never manufacture missing records or silently overwrite.
            m.next = m
                .next
                .checked_add(RECORDS_PER_CHUNK as u64 - m.next % RECORDS_PER_CHUNK as u64)
                .ok_or(Error::Invalid)?;
            m.gaps = m.gaps.saturating_add(u64::from(m.slots & (1 << slot) != 0));
            save_meta(b, &m)?;
            return append(b, r);
        }
        None => {}
    }
    let suppressed = if r.outcome == "suppressed" {
        u64::from(r.count)
    } else {
        0
    };
    target.records.push(r);
    target.end += 1;
    let raw = encoded(&target);
    if target.records.len() > RECORDS_PER_CHUNK || raw.len() > MAX_CHUNK_BYTES {
        return Err(Error::Invalid);
    }
    let next = m.next.checked_add(1).ok_or(Error::Invalid)?;
    // Bound is structural, checked before ANY data write, including growth in
    // decimal counters. No optimistic post-write quota check.
    m.pending = true;
    save_meta(b, &m)?;
    b.set(&key(slot), &raw).map_err(|_| Error::Storage)?;
    m.next = next;
    m.slots |= 1 << slot;
    m.suppressed = m.suppressed.saturating_add(suppressed);
    m.pending = false;
    save_meta(b, &m)
}
pub fn next_correlation<B: Backend>(b: &mut B) -> Result<u64, Error> {
    let mut m = load_meta(b)?;
    m.correlation = m.correlation.checked_add(1).ok_or(Error::Invalid)?;
    save_meta(b, &m)?;
    Ok(m.correlation)
}
pub fn note_gap<B: Backend>(b: &mut B) -> Result<(), Error> {
    note_gaps(b, 1)
}
pub fn note_gaps<B: Backend>(b: &mut B, count: u64) -> Result<(), Error> {
    let mut m = load_meta(b)?;
    m.gaps = m.gaps.saturating_add(count);
    m.write_failures = m.write_failures.saturating_add(count);
    save_meta(b, &m)
}
/// Priority reclamation for an essential runtime write proven to exceed quota.
/// Keep a loss marker and monotonic cursor. Never touch other namespaces.
pub fn reclaim<B: Backend>(b: &mut B) -> Result<(), Error> {
    let raw = raw_meta(b)?;
    let s = scan(b)?;
    let mut m = recover(raw.as_deref(), &s);
    m.pending = true;
    m.gaps = m.gaps.saturating_add(1);
    // The marker normally replaces an existing value. If quota prevents even
    // that, delete data first; missing metadata/chunks are detected on recovery.
    let _ = save_meta(b, &m);
    m.truncated = m.truncated.saturating_add(
        s.chunks
            .iter()
            .map(|(_, c)| c.records.len() as u64)
            .sum::<u64>(),
    );
    for slot in 0..CHUNK_SLOTS {
        b.delete(&key(slot)).map_err(|_| Error::Storage)?;
    }
    m.slots = 0;
    m.next = m
        .next
        .checked_add(RECORDS_PER_CHUNK as u64 - m.next % RECORDS_PER_CHUNK as u64)
        .ok_or(Error::Invalid)?;
    m.pending = false;
    save_meta(b, &m)
}
/// Read-only, content-token guarded export. Page>0 requires the token returned by
/// page0; append, eviction, gaps or expiry invalidate it explicitly.
pub fn export_at<B: Backend>(
    b: &mut B,
    at: i64,
    page: usize,
    per_page: usize,
    expected: Option<&str>,
) -> Result<Export, Error> {
    if at <= 0 || per_page == 0 || per_page > 100 || (page > 0 && expected.is_none()) {
        return Err(Error::Invalid);
    }
    let raw = raw_meta(b)?;
    let s = scan(b)?;
    let m = recover(raw.as_deref(), &s);
    let used_bytes = s.bytes + raw.as_ref().map_or(0, |r| META.len() + r.len());
    let all: Vec<Record> = s.chunks.into_iter().flat_map(|(_, c)| c.records).collect();
    let total_before = all.len();
    let all: Vec<Record> = all
        .into_iter()
        .filter(|r| r.last_at >= at.saturating_sub(MAX_AGE))
        .collect();
    let snapshot = format!(
        "{:016x}",
        hash(encoded(&(&m, &all, used_bytes, per_page)).as_bytes())
    );
    if expected.is_some_and(|v| v != snapshot) {
        return Err(Error::SnapshotChanged);
    }
    let start = page
        .checked_mul(per_page)
        .ok_or(Error::Invalid)?
        .min(all.len());
    let end = start.saturating_add(per_page).min(all.len());
    Ok(Export {
        oldest: all.iter().map(|r| r.first_at).min(),
        newest: all.iter().map(|r| r.last_at).max(),
        used_bytes,
        truncation_count: m.truncated,
        suppression_count: m.suppressed,
        write_failure_count: m.write_failures,
        gap_count: m.gaps,
        complete: m.gaps == 0 && m.truncated == 0 && m.suppressed == 0,
        counters_exact: m.gaps == 0,
        total: all.len(),
        records: all[start..end].to_vec(),
        page,
        per_page,
        snapshot,
        has_more: end < all.len(),
        expired_records: total_before - all.len(),
    })
}
/// Compatibility for host-neutral tests; production must supply its current clock.
pub fn export<B: Backend>(b: &mut B, page: usize, per_page: usize) -> Result<Export, Error> {
    export_at(b, 1, page, per_page, None)
}
#[cfg(test)]
#[path = "diagnostics_tests.rs"]
mod tests;
