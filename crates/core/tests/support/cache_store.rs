//! Compare the independent Reader with persisted, replayed facts at every record boundary.
use agent_session_core::{claude::parse::TranscriptParser, index::Db, model::*};

pub fn check(bytes: &[u8], conversation: &Conversation) {
    let expected: Vec<_> = conversation.items.iter().flat_map(|item| match item {
        ConvItem::Turn(t) => t.cache_breaks.iter().map(|b| (t.number, b.clone())).collect(),
        _ => vec![],
    }).collect();
    let db = Db::open_in_memory().unwrap();
    let project = db.upsert_project("sample", None).unwrap();
    db.upsert_session(&SessionSummary::transcript("sample", "sample", std::path::Path::new("sample.jsonl")), project).unwrap();
    let full = TranscriptParser::new("sample").feed(bytes, 0);
    db.insert_cache_events("sample", &full.cache_events).unwrap();
    db.refresh_cache_breaks("sample").unwrap();
    let breaks = db.cache_breaks("sample").unwrap();
    let calls = db.cache_calls("sample").unwrap();
    assert_eq!(breaks.iter().map(|b| (b.turn, b.rebuilt.clone())).collect::<Vec<_>>(), expected);
    let mut splits: Vec<_> = bytes.iter().enumerate().filter(|(_, b)| **b == b'\n').map(|(i, _)| i + 1).collect();
    // A tail ending in a partial record must be replayed from consumed, not from EOF.
    splits.push(bytes.len() / 2);
    for split in splits {
        db.delete_cache_facts("sample").unwrap();
        let first = TranscriptParser::new("sample").feed(&bytes[..split], 0);
        db.insert_cache_events("sample", &first.cache_events).unwrap();
        db.refresh_cache_breaks("sample").unwrap();
        let offset = first.consumed as usize;
        let second = TranscriptParser::new("sample").feed(&bytes[offset..], offset as u64);
        db.insert_cache_events("sample", &second.cache_events).unwrap();
        db.refresh_cache_breaks("sample").unwrap();
        assert_eq!(db.cache_events("sample").unwrap(), full.cache_events, "events split {split}");
        assert_eq!(db.cache_calls("sample").unwrap(), calls, "calls split {split}");
        assert_eq!(db.cache_breaks("sample").unwrap(), breaks, "breaks split {split}");
    }
}
