use super::*;
use std::collections::BTreeMap;
#[derive(Default)]
struct Mem {
    values: BTreeMap<String, String>,
    gets: usize,
    sets: usize,
    deletes: usize,
    fail_at: Option<usize>,
    peak: usize,
}
impl Mem {
    fn bytes(&self) -> usize {
        self.values.iter().map(|(k, v)| k.len() + v.len()).sum()
    }
}
impl Backend for Mem {
    type Error = String;
    fn get(&mut self, k: &str) -> Result<Option<String>, String> {
        self.gets += 1;
        Ok(self.values.get(k).cloned())
    }
    fn set(&mut self, k: &str, v: &str) -> Result<(), String> {
        self.sets += 1;
        if self.fail_at == Some(self.sets) {
            return Err("injected".into());
        }
        let size = self
            .values
            .iter()
            .filter(|(key, _)| key.as_str() != k)
            .map(|(k, v)| k.len() + v.len())
            .sum::<usize>()
            + k.len()
            + v.len();
        if size > 1_048_576 {
            return Err("quota".into());
        }
        self.values.insert(k.into(), v.into());
        self.peak = self.peak.max(size);
        Ok(())
    }
    fn delete(&mut self, k: &str) -> Result<(), String> {
        self.deletes += 1;
        self.values.remove(k);
        Ok(())
    }
    fn list(&mut self, _: &str) -> Result<Vec<String>, String> {
        panic!("fixed journal must not enumerate host inventory")
    }
}
fn r(at: i64) -> Record {
    Record {
        received_at: at,
        provider_at: Some(at),
        mat: Some("mat_aabbccddeeff0011".into()),
        event: "bed_in".into(),
        stage: "decision".into(),
        outcome: "duplicate_or_stale".into(),
        correlation: at as u64,
        count: 1,
        first_at: at,
        last_at: at,
    }
}
#[test]
fn realistic_thirty_day_model_measures_exact_bytes_and_export_pages() {
    let mut b = Mem::default();
    let start = 1_792_200_000;
    for i in 0..1080 {
        append(&mut b, r(start + i * 2400)).unwrap();
    }
    let e = export_at(&mut b, start + 1079 * 2400, 0, 100, None).unwrap();
    assert_eq!(e.total, 1080);
    assert_eq!(e.used_bytes, b.bytes());
    assert!(b.peak <= MAX_BYTES);
    assert!(e.used_bytes + 65_536 + 65_536 + 4096 < 1_048_576);
    let before = b.values.clone();
    let mut count = e.records.len();
    for page in 1..11 {
        count += export_at(&mut b, start + 1079 * 2400, page, 100, Some(&e.snapshot))
            .unwrap()
            .records
            .len();
    }
    assert_eq!(count, 1080);
    assert_eq!(before, b.values);
    println!(
        "1080 real-epoch records: {} key+value bytes; static cap {} bytes",
        e.used_bytes, MAX_BYTES
    );
}
#[test]
fn hot_append_is_two_reads_and_three_writes_no_event_merging() {
    let mut b = Mem::default();
    append(&mut b, r(1000)).unwrap();
    let (gets, sets) = (b.gets, b.sets);
    append(&mut b, r(1000)).unwrap();
    assert_eq!(b.gets - gets, 2);
    assert_eq!(b.sets - sets, 3);
    assert_eq!(export(&mut b, 0, 100).unwrap().total, 2);
}
#[test]
fn capacity_bounds_before_every_write_and_counts_evicted_records() {
    let mut b = Mem::default();
    for _ in 0..MAX_RECORDS + 1 {
        append(&mut b, r(1_792_200_000)).unwrap();
        assert!(b.bytes() <= MAX_BYTES);
    }
    let e = export(&mut b, 0, 100).unwrap();
    assert_eq!(e.total, MAX_RECORDS + 1 - RECORDS_PER_CHUNK);
    assert_eq!(e.truncation_count, RECORDS_PER_CHUNK as u64);
    assert!(!e.complete);
}
#[test]
fn partial_age_pruning_preserves_cursor_and_corruption_is_recoverable() {
    let mut b = Mem::default();
    append(&mut b, r(1000)).unwrap();
    append(&mut b, r(1000 + MAX_AGE - 1)).unwrap();
    append(&mut b, r(1000 + MAX_AGE + 86400)).unwrap();
    assert!(!b.values[&key(0)].contains("\"received_at\":1000,"));
    b.values.remove(META);
    append(&mut b, r(1000 + MAX_AGE + 86401)).unwrap();
    assert_eq!(export(&mut b, 0, 100).unwrap().total, 3);
    b.values.insert(key(0), "{".into());
    assert_eq!(
        append(&mut b, r(1000 + MAX_AGE + 86402)),
        Err(Error::Corrupt)
    );
    append(&mut b, r(1000 + MAX_AGE + 86403)).unwrap();
    let e = export(&mut b, 0, 100).unwrap();
    assert_eq!(e.total, 1);
    assert!(!e.counters_exact);
    assert!(!e.complete);
}

#[test]
fn invalid_fields_are_rejected_without_io() {
    let mut b = Mem::default();
    for field in 0..5 {
        let mut x = r(1000);
        match field {
            0 => x.outcome = "private_secret".into(),
            1 => x.mat = Some("mat_private_secret".into()),
            2 => x.provider_at = None,
            3 => x.first_at = 2000,
            _ => x.count = 2,
        };
        assert_eq!(append(&mut b, x), Err(Error::Invalid));
    }
    assert_eq!(b.gets + b.sets, 0);
}
#[test]
fn age_cleanup_and_rollback_preserve_separate_records() {
    let mut b = Mem::default();
    append(&mut b, r(1000)).unwrap();
    append(&mut b, r(900)).unwrap();
    assert_eq!(export_at(&mut b, 1100, 0, 100, None).unwrap().total, 2);
    let before = b.values.clone();
    assert_eq!(
        export_at(&mut b, MAX_AGE + 1101, 0, 100, None)
            .unwrap()
            .total,
        0
    );
    assert_eq!(b.values, before);
    append(&mut b, r(MAX_AGE + 1101)).unwrap();
    let e = export_at(&mut b, MAX_AGE + 1101, 0, 100, None).unwrap();
    assert_eq!(e.total, 1);
    assert_eq!(e.truncation_count, 2);
}
#[test]
fn lost_or_corrupt_metadata_recovers_without_overwriting_prior_records() {
    for corrupt in [false, true] {
        let mut b = Mem::default();
        append(&mut b, r(1000)).unwrap();
        append(&mut b, r(1001)).unwrap();
        if corrupt {
            b.values.insert(META.into(), "{".into());
        } else {
            b.values.remove(META);
        }
        append(&mut b, r(1002)).unwrap();
        let e = export(&mut b, 0, 100).unwrap();
        assert_eq!(e.total, 3);
        assert!(e.gap_count > 0);
        assert!(!e.complete);
    }
}
#[test]
fn every_append_write_failure_preserves_history_and_marks_partial_commit() {
    for step in 1..=3 {
        let mut b = Mem::default();
        append(&mut b, r(1000)).unwrap();
        b.fail_at = Some(b.sets + step);
        assert_eq!(append(&mut b, r(1001)), Err(Error::Storage));
        b.fail_at = None;
        let e = export(&mut b, 0, 100).unwrap();
        assert_eq!(e.total, if step == 3 { 2 } else { 1 });
        if step > 1 {
            assert!(e.gap_count > 0);
        }
        note_gap(&mut b).unwrap();
        append(&mut b, r(1002)).unwrap();
        let e = export(&mut b, 0, 100).unwrap();
        assert!(e.gap_count > 0);
        assert!(e.records.iter().any(|r| r.received_at == 1000));
    }
}
#[test]
fn corrupt_or_missing_chunk_is_not_complete_history() {
    for corrupt in [false, true] {
        let mut b = Mem::default();
        append(&mut b, r(1000)).unwrap();
        if corrupt {
            b.values.insert(key(0), "{".into());
        } else {
            b.values.remove(&key(0));
        }
        let e = export(&mut b, 0, 100).unwrap();
        assert_eq!(e.total, 0);
        assert!(e.gap_count > 0);
        assert!(!e.complete);
    }
}
#[test]
fn export_requires_and_invalidates_snapshot_without_writes() {
    let mut b = Mem::default();
    append(&mut b, r(1000)).unwrap();
    let e = export_at(&mut b, 1000, 0, 1, None).unwrap();
    assert_eq!(export_at(&mut b, 1000, 1, 1, None), Err(Error::Invalid));
    append(&mut b, r(1001)).unwrap();
    assert_eq!(
        export_at(&mut b, 1001, 1, 1, Some(&e.snapshot)),
        Err(Error::SnapshotChanged)
    );
    let e = export_at(&mut b, 1001, 0, 1, None).unwrap();
    assert_eq!(
        export_at(&mut b, MAX_AGE + 1002, 1, 1, Some(&e.snapshot)),
        Err(Error::SnapshotChanged)
    );
}
#[test]
fn essential_quota_reclaim_preserves_other_state_and_loss_marker() {
    let mut b = Mem::default();
    b.set("presence", &"p".repeat(65_536)).unwrap();
    for _ in 0..1080 {
        append(&mut b, r(1_792_200_000)).unwrap();
    }
    let journal = b.bytes() - "presence".len() - 65_536;
    let size = 1_048_576 - b.bytes() - "other".len() - 20;
    b.set("other", &"o".repeat(size)).unwrap();
    assert!(b.set("essential", &"e".repeat(65_536)).is_err());
    let before = b.values["presence"].clone();
    reclaim(&mut b).unwrap();
    b.set("essential", &"e".repeat(65_536)).unwrap();
    assert_eq!(b.values["presence"], before);
    assert!(journal > 65_536);
    let e = export(&mut b, 0, 100).unwrap();
    assert_eq!(e.total, 0);
    assert_eq!(e.truncation_count, 1080);
    assert!(!e.complete);
}
#[test]
fn correlations_are_reserved_across_recreation_and_summaries_count_suppression() {
    let mut b = Mem::default();
    let a = next_correlation(&mut b).unwrap();
    let z = next_correlation(&mut b).unwrap();
    assert!(z > a);
    let summary = Record {
        mat: None,
        provider_at: None,
        event: "webhook".into(),
        stage: "receipt".into(),
        outcome: "suppressed".into(),
        count: 999,
        first_at: 900,
        ..r(1000)
    };
    append(&mut b, summary).unwrap();
    let e = export(&mut b, 0, 100).unwrap();
    assert_eq!(e.suppression_count, 999);
    assert_eq!(e.records[0].first_at, 900);
    assert_eq!(e.records[0].last_at, 1000);
    assert!(!e.complete);
}
