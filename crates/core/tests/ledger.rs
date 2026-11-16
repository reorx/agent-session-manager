mod common;
use common::*;
use agent_session_core::{index::{Db, Indexer, SCHEMA_VERSION}, ledger::{Ledger, LedgerBatch, LEDGER_VERSION}, claude::paths::ledger_path, model::UsageRow};
use rusqlite::Connection;

fn scan(sb: &Sandbox, db: &mut Db) { Indexer::new(sb.claude.clone(),None).scan(db,&mut |_|{}).unwrap(); }
fn ledger(sb: &Sandbox) -> Connection { Connection::open(ledger_path(&sb.db_path())).unwrap() }
fn count(c: &Connection, table: &str) -> i64 { c.query_row(&format!("SELECT count(*) FROM {table}"),[],|r|r.get(0)).unwrap() }
fn snapshot(sb: &Sandbox) -> Vec<(String,i64,i64,i64,i64,i64,i64,i64)> {
    ledger(sb).prepare("SELECT session_id,count(*),sum(input),sum(cache_write_5m),sum(cache_write_1h),sum(cache_read),sum(output),sum(web_search) FROM responses GROUP BY session_id ORDER BY session_id").unwrap()
        .query_map([],|r|Ok((r.get(0)?,r.get(1)?,r.get(2)?,r.get(3)?,r.get(4)?,r.get(5)?,r.get(6)?,r.get(7)?))).unwrap().map(Result::unwrap).collect()
}
fn row(id: &str) -> UsageRow { UsageRow {msg_id:id.into(),model:"claude-sonnet-5".into(),ts:3600000,input:100,cache_write_5m:20,cache_write_1h:30,cache_read:40,output:50,web_search:1,fast:false,cost_usd:None} }
fn remove_s1(sb: &Sandbox) {
    std::fs::remove_file(sb.transcript(ALPHA,S1)).unwrap();
    std::fs::remove_dir_all(sb.root().join("projects").join(ALPHA).join(S1)).unwrap();
}
#[test]
fn unchecked_transactions_disable_autocommit() {
    let c=Connection::open_in_memory().unwrap();
    assert!(c.is_autocommit());
    let tx=rusqlite::Transaction::new_unchecked(&c,rusqlite::TransactionBehavior::Immediate).unwrap();
    assert!(!c.is_autocommit()); tx.commit().unwrap(); assert!(c.is_autocommit());
}
#[test]
fn the_ledger_schema_is_what_its_migrations_build() {
    let sb=sandbox(); let path=ledger_path(&sb.db_path());
    drop(Ledger::open(&path).unwrap());
    let a=Connection::open(path).unwrap();
    let b=Connection::open_in_memory().unwrap();
    b.execute_batch(include_str!("../src/ledger.sql")).unwrap();
    let schema=|c:&Connection| c.prepare("SELECT type,name,sql FROM sqlite_master ORDER BY name").unwrap().query_map([],|r|Ok((r.get::<_,String>(0)?,r.get::<_,String>(1)?,r.get::<_,Option<String>>(2)?))).unwrap().map(Result::unwrap).collect::<Vec<_>>();
    assert_eq!(schema(&a),schema(&b));
    assert_eq!(a.query_row("PRAGMA user_version",[],|r|r.get::<_,i32>(0)).unwrap(),LEDGER_VERSION);
}
#[test]
fn ledger_upserts_keep_the_first_timestamp_and_refresh_all_price_groups() {
    let l=Ledger::open_in_memory().unwrap();
    let index=Db::open_in_memory().unwrap();
    let mut b=LedgerBatch::default();
    b.responses.push(("s".into(),"".into(),row("a")));
    b.responses.push(("s".into(),"sub".into(),row("b")));
    l.write(&b,index.conn()).unwrap();
    b.responses[0].2.ts=7200000; b.responses[0].2.input=300000; b.responses[0].2.fast=true;
    l.write(&b,index.conn()).unwrap();
    let u=l.usage(None).unwrap();
    assert_eq!(l.response_counts().unwrap()["s"],2);
    assert_eq!(u[0].hours.len(),2);
    assert!(u[0].hours.iter().all(|h|h.hour==1));
    assert!(u[0].hours.iter().any(|h|h.fast && h.long_prompt && !h.sub));
    assert_eq!(u[0].session.project_key,"");
    assert!(l.usage(Some(&[])).unwrap().is_empty());
}
#[test]
fn a_scan_writes_responses_sessions_projects_and_hourly_rows_to_the_ledger() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    assert_eq!(count(&ledger(&sb),"responses"),count(db.conn(),"api_usage"));
    let u=db.ledger_usage(Some(&[S1.into()])).unwrap().unwrap();
    let s=db.session(S1).unwrap().unwrap();
    assert_eq!(u.len(),1); assert_eq!(u[0].session.project_key,ALPHA);
    assert_eq!(u[0].session.title,s.display_title()); assert_eq!(u[0].session.entrypoint,s.entrypoint);
    assert_eq!(u[0].session.project_path,s.project_path);
    for s in db.ledger_usage(None).unwrap().unwrap() {
        let rows=db.usage(&s.session.id).unwrap();
        let mut tokens=agent_session_core::model::Tokens::default();
        for r in &rows {tokens.add(&r.tokens());}
        let mut hourly=agent_session_core::model::Tokens::default();
        for h in &s.hours {hourly.add(&h.tokens);}
        assert_eq!(tokens,hourly);
        assert_eq!(s.hours.iter().map(|h|h.responses).sum::<u64>(),rows.len() as u64);
    }
}
#[test]
fn a_response_cut_between_two_scans_is_in_the_ledger_once() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let path=sb.transcript(ALPHA,S2);
    let reply=r#"{"type":"assistant","uuid":"cut","timestamp":"2026-11-16T00:00:00Z","message":{"id":"cut","model":"claude-sonnet-5","content":[{"type":"text","text":"hi"}],"usage":{"input_tokens":100,"output_tokens":20}}}
"#;
    append(&path,reply); scan(&sb,&mut db); let before=snapshot(&sb);
    append(&path,reply); scan(&sb,&mut db); assert_eq!(snapshot(&sb),before);
    assert_eq!(ledger(&sb).query_row("SELECT count(*) FROM responses WHERE msg_id='cut'",[],|r|r.get::<_,i64>(0)).unwrap(),1);
}
#[test]
fn deleting_transcripts_then_scanning_and_rebuilding_leaves_the_ledger_unchanged() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=snapshot(&sb); remove_s1(&sb); scan(&sb,&mut db);
    Indexer::new(sb.claude.clone(),None).rebuild(&mut db,&mut |_|{}).unwrap();
    assert_eq!(snapshot(&sb),before); assert!(db.usage(S1).unwrap().is_empty());
}
#[test]
fn a_subagent_deleted_on_its_own_leaves_the_index_but_stays_in_the_ledger() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=snapshot(&sb); let n=db.usage(S1).unwrap().len();
    std::fs::remove_dir_all(sb.root().join("projects").join(ALPHA).join(S1)).unwrap(); scan(&sb,&mut db);
    assert!(db.usage(S1).unwrap().len()<n); assert_eq!(snapshot(&sb),before);
}
#[test]
fn a_rewritten_transcript_does_not_remove_ledger_rows() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=snapshot(&sb); let n=db.usage(S1).unwrap().len();
    std::fs::write(sb.transcript(ALPHA,S1),"").unwrap(); scan(&sb,&mut db);
    assert!(db.usage(S1).unwrap().len()<n); assert_eq!(snapshot(&sb),before);
}
#[test]
fn an_index_of_another_version_is_copied_into_the_ledger_before_it_is_rebuilt() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=db.ledger_usage(None).unwrap().unwrap();
    ledger(&sb).execute_batch("DELETE FROM responses; DELETE FROM usage_hourly; DELETE FROM sessions; DELETE FROM projects;").unwrap();
    db.set_user_version_for_test(SCHEMA_VERSION-1); drop(db); remove_s1(&sb);
    let db=Db::open(&sb.db_path()).unwrap();
    assert_eq!(db.ledger_usage(None).unwrap().unwrap(),before); assert_eq!(count(db.conn(),"sessions"),0);
}
#[test]
fn an_old_index_without_usage_tables_is_rebuilt_without_failing() {
    let sb=sandbox(); let c=Connection::open(sb.db_path()).unwrap();
    c.execute_batch("CREATE TABLE sessions (id TEXT); PRAGMA user_version=1;").unwrap(); drop(c);
    let db=Db::open(&sb.db_path()).unwrap(); assert!(db.ledger_usage(None).unwrap().unwrap().is_empty());
}
#[test]
fn a_corrupt_index_is_deleted_and_the_ledger_is_left_alone() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=snapshot(&sb); drop(db);
    std::fs::write(sb.db_path(),b"garbage").unwrap();
    let db=Db::open(&sb.db_path()).unwrap();
    assert_eq!(snapshot(&sb),before); assert_eq!(count(db.conn(),"sessions"),0);
}
#[test]
fn a_corrupt_ledger_is_renamed_aside_and_refilled_from_the_index() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=snapshot(&sb); let states=count(db.conn(),"index_state"); drop(db);
    std::fs::write(ledger_path(&sb.db_path()),b"garbage").unwrap();
    let db=Db::open(&sb.db_path()).unwrap();
    assert_eq!(snapshot(&sb),before); assert_eq!(count(db.conn(),"index_state"),states);
    assert!(db.session(S1).unwrap().is_some());
    assert!(std::fs::read_dir(sb.tmp.path()).unwrap().any(|e|e.unwrap().file_name().to_string_lossy().starts_with("usage-ledger.corrupt-")));
}
#[test]
fn rows_the_ledger_lacks_are_copied_from_the_index_on_open() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=snapshot(&sb); drop(db);
    ledger(&sb).execute("DELETE FROM responses WHERE session_id=?1",[S1]).unwrap();
    Ledger::open(&ledger_path(&sb.db_path())).unwrap().refresh_hourly(S1).unwrap();
    let _db=Db::open(&sb.db_path()).unwrap(); assert_eq!(snapshot(&sb),before);
}
#[test]
fn the_first_open_backfills_sessions_whose_transcripts_are_gone() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=snapshot(&sb); remove_s1(&sb); scan(&sb,&mut db); drop(db);
    std::fs::remove_file(ledger_path(&sb.db_path())).unwrap();
    let _db=Db::open(&sb.db_path()).unwrap(); assert_eq!(snapshot(&sb),before);
}
#[test]
fn a_ledger_of_a_newer_version_is_left_alone_and_the_index_still_works() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db); drop(db);
    ledger(&sb).pragma_update(None,"user_version",LEDGER_VERSION+1).unwrap();
    let before=std::fs::read(ledger_path(&sb.db_path())).unwrap();
    let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    assert!(db.ledger_usage(None).unwrap().is_none()); drop(db);
    assert_eq!(std::fs::read(ledger_path(&sb.db_path())).unwrap(),before);
}
#[test]
fn two_indexes_in_one_directory_share_the_ledger_without_double_counting() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=snapshot(&sb);
    let mut cli=Db::open(&sb.tmp.path().join("index-cli.sqlite")).unwrap(); scan(&sb,&mut cli);
    assert_eq!(snapshot(&sb),before);
}
#[test]
fn a_failed_scan_leaves_nothing_in_the_ledger() {
    let db=Db::open_in_memory().unwrap();
    let err=db.transaction::<()>(|db|{ db.insert_usage("s","",&[row("a")])?; anyhow::bail!("deliberate failure") }).unwrap_err();
    assert!(err.to_string().contains("deliberate failure"));
    db.upsert_project("p",None).unwrap(); // would flush stale pending
    assert!(db.ledger_usage(None).unwrap().unwrap().is_empty());
    assert_eq!(count(db.conn(),"api_usage"),0);
}
#[test]
fn session_info_survives_a_rebuild_that_brings_the_session_back_from_history() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=db.ledger_usage(Some(&[S1.into()])).unwrap().unwrap();
    remove_s1(&sb); Indexer::new(sb.claude.clone(),None).rebuild(&mut db,&mut |_|{}).unwrap();
    assert_eq!(db.session(S1).unwrap().unwrap().source,agent_session_core::model::Source::History);
    assert_eq!(db.ledger_usage(Some(&[S1.into()])).unwrap().unwrap(),before);
}

#[test]
fn a_failed_ledger_write_rolls_back_the_index_and_clears_pending() {
    let sb=sandbox(); let db=Db::open(&sb.db_path()).unwrap();
    db.set_meta("sentinel","keep").unwrap();
    ledger(&sb).execute_batch("CREATE TRIGGER fail_write BEFORE INSERT ON responses BEGIN SELECT RAISE(ABORT,'deliberate ledger failure'); END;").unwrap();
    let err=db.transaction(|db|db.insert_usage("s","",&[row("a")])).unwrap_err();
    assert!(format!("{err:#}").contains("deliberate ledger failure"));
    assert_eq!(count(db.conn(),"api_usage"),0);
    ledger(&sb).execute_batch("DROP TRIGGER fail_write;").unwrap();
    db.upsert_project("p",None).unwrap();
    assert!(db.ledger_usage(None).unwrap().unwrap().is_empty());
    assert_eq!(db.meta("sentinel").unwrap().as_deref(),Some("keep"));
}

#[test]
fn ledger_errors_during_open_do_not_delete_the_index() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db); drop(db);
    ledger(&sb).execute_batch("DELETE FROM responses; DELETE FROM usage_hourly; CREATE TRIGGER fail_write BEFORE INSERT ON responses BEGIN SELECT RAISE(ABORT,'deliberate ledger failure'); END;").unwrap();
    let err=match Db::open(&sb.db_path()) {Ok(_)=>panic!("write must fail"),Err(e)=>e};
    assert!(format!("{err:#}").contains("deliberate ledger failure"));
    assert!(count(&Connection::open(sb.db_path()).unwrap(),"index_state")>0);
}

#[test]
#[ignore = "measurement uses an explicitly supplied scratch copy, never the live index"]
fn measure_backfill_and_reopen() {
    let path=std::path::PathBuf::from(std::env::var("ASM_LEDGER_BENCH_INDEX").expect("scratch index path"));
    assert!(path.to_string_lossy().contains("/tmp/2026-11-12-usage-stats/usage-p1-core/"));
    let start=std::time::Instant::now();
    let db=Db::open(&path).unwrap();
    let backfill=start.elapsed();
    let usage=db.ledger_usage(None).unwrap().unwrap();
    let calls:u64=usage.iter().flat_map(|s|&s.hours).map(|h|h.responses).sum();
    let hours:usize=usage.iter().map(|s|s.hours.len()).sum();
    drop(db);
    let bytes=std::fs::metadata(ledger_path(&path)).unwrap().len();
    let start=std::time::Instant::now();
    let db=Db::open(&path).unwrap();
    let reopen=start.elapsed();
    assert_eq!(db.ledger_usage(None).unwrap().unwrap(),usage);
    println!("backfill_ms={:.3} ledger_bytes={bytes} reopen_ms={:.3} sessions={} responses={calls} hours={hours}",
        backfill.as_secs_f64()*1000.,reopen.as_secs_f64()*1000.,usage.len());
}

#[test]
fn damage_in_ledger_tables_does_not_get_classified_as_index_damage() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=snapshot(&sb); let states=count(db.conn(),"index_state"); drop(db);
    ledger(&sb).execute_batch("PRAGMA writable_schema=ON; UPDATE sqlite_master SET rootpage=2147483647 WHERE name='usage_hourly'; PRAGMA writable_schema=OFF;").unwrap();
    let db=Db::open(&sb.db_path()).unwrap();
    assert_eq!(snapshot(&sb),before);
    assert_eq!(count(db.conn(),"index_state"),states);
}

#[test]
fn simultaneous_indexes_can_initialize_the_same_ledger() {
    let sb=sandbox();
    let barrier=std::sync::Arc::new(std::sync::Barrier::new(2));
    std::thread::scope(|scope| {
        let handles:Vec<_>=["index.sqlite","index-cli.sqlite"].into_iter().map(|name| {
            let path=sb.tmp.path().join(name); let barrier=barrier.clone();
            scope.spawn(move || {barrier.wait(); Db::open(&path).map(|_|())})
        }).collect();
        for h in handles { h.join().unwrap().unwrap(); }
    });
    assert_eq!(count(&ledger(&sb),"responses"),0);
}

#[test]
fn refreshing_hourly_outside_a_batch_is_atomic() {
    let sb=sandbox(); let path=ledger_path(&sb.db_path());
    let l=Ledger::open(&path).unwrap(); let index=Db::open_in_memory().unwrap();
    let mut batch=LedgerBatch::default(); batch.responses.push(("s".into(),"".into(),row("a")));
    l.write(&batch,index.conn()).unwrap();
    ledger(&sb).execute_batch("CREATE TRIGGER fail_hourly BEFORE INSERT ON usage_hourly BEGIN SELECT RAISE(ABORT,'hourly failure'); END;").unwrap();
    let e=l.refresh_hourly("s").unwrap_err();
    assert!(format!("{e:#}").contains("hourly failure"));
    assert_eq!(l.response_counts().unwrap()["s"],1);
}

#[test]
fn corruption_during_a_write_recovers_uncommitted_index_usage() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=count(db.conn(),"api_usage");
    ledger(&sb).execute_batch("PRAGMA writable_schema=ON; UPDATE sqlite_master SET rootpage=2147483647 WHERE name='responses'; PRAGMA schema_version=9001; PRAGMA writable_schema=OFF;").unwrap();
    db.transaction(|db| {
        db.insert_usage(S1,"",&[row("after-damage")])?;
        db.refresh_session_usage(S1)
    }).unwrap();
    assert_eq!(count(db.conn(),"api_usage"),before+1);
    assert_eq!(count(&ledger(&sb),"responses"),before+1);
    assert!(db.session(S1).unwrap().is_some());
}

#[test]
fn the_ledger_commits_before_the_index_and_retry_does_not_double_count() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let before=count(db.conn(),"api_usage");
    let err=db.transaction(|db| {
        db.conn().execute_batch("PRAGMA defer_foreign_keys=ON; INSERT INTO sessions(id,project_id) VALUES('invalid',-1);")?;
        db.insert_usage(S1,"",&[row("before-commit")])?;
        db.refresh_session_usage(S1)
    }).unwrap_err();
    assert!(format!("{err:#}").contains("FOREIGN KEY constraint failed"));
    assert_eq!(count(db.conn(),"api_usage"),before);
    assert_eq!(count(&ledger(&sb),"responses"),before+1);
    db.transaction(|db| {
        db.insert_usage(S1,"",&[row("before-commit")])?;
        db.refresh_session_usage(S1)
    }).unwrap();
    assert_eq!(count(&ledger(&sb),"responses"),before+1);
    assert_eq!(count(db.conn(),"api_usage"),before+1);
}

#[test]
fn session_metadata_is_available_even_before_its_first_response() {
    let sb=sandbox(); let mut db=Db::open(&sb.db_path()).unwrap(); scan(&sb,&mut db);
    let l=Ledger::open_in_memory().unwrap();
    let mut batch=LedgerBatch::default(); batch.sessions.insert(S1.into());
    l.write(&batch,db.conn()).unwrap();
    let usage=l.usage(Some(&[S1.into()])).unwrap();
    assert_eq!(usage.len(),1);
    assert_eq!(usage[0].session.id,S1);
    assert!(usage[0].hours.is_empty());
}

#[test]
fn a_failed_recovery_cannot_silently_disable_future_ledger_writes() {
    let sb=sandbox();
    let dir=sb.tmp.path().join("store");
    let path=dir.join("index.sqlite");
    let mut db=Db::open(&path).unwrap(); scan(&sb,&mut db);
    Connection::open(ledger_path(&path)).unwrap().execute_batch(
        "PRAGMA writable_schema=ON; UPDATE sqlite_master SET rootpage=2147483647 WHERE name='responses'; PRAGMA schema_version=9001; PRAGMA writable_schema=OFF;"
    ).unwrap();
    let moved=sb.tmp.path().join("moved-store");
    std::fs::rename(&dir,&moved).unwrap();
    std::fs::write(&dir,"blocks reopening the ledger").unwrap();
    let first=db.transaction(|db|db.insert_usage(S1,"",&[row("recovery-fails")]));
    assert!(format!("{:#}",first.unwrap_err()).contains("recreate ledger"));
    std::fs::remove_file(&dir).unwrap();
    std::fs::rename(&moved,&dir).unwrap();
    let second=db.transaction(|db|db.insert_usage(S1,"",&[row("must-not-skip")]));
    assert!(format!("{:#}",second.unwrap_err()).contains("ledger"));
    assert!(!db.usage(S1).unwrap().iter().any(|r|r.msg_id=="must-not-skip"));
}

#[test]
fn shared_ledger_busy_remains_classifiable() {
    let sb = sandbox();
    let db = Db::open(&sb.db_path()).unwrap();
    let lock = ledger(&sb);
    lock.execute_batch("BEGIN IMMEDIATE").unwrap();
    let error = db.transaction(|db| db.insert_usage("review", "", &[row("busy")])).unwrap_err();
    assert!(agent_session_core::index::is_busy(&error), "{error:#}");
    assert_eq!(count(db.conn(), "api_usage"), 0);
}

#[test]
fn ledger_busy_during_open_remains_classifiable_and_keeps_the_index() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let states = count(db.conn(), "index_state");
    drop(db);
    let lock = ledger(&sb);
    lock.execute_batch("DELETE FROM responses; DELETE FROM usage_hourly; BEGIN IMMEDIATE;").unwrap();
    let error = match Db::open(&sb.db_path()) {
        Ok(_) => panic!("backfill must wait for the ledger lock"),
        Err(error) => error,
    };
    assert!(agent_session_core::index::is_busy(&error), "{error:#}");
    assert_eq!(count(&Connection::open(sb.db_path()).unwrap(), "index_state"), states);
}

#[test]
fn a_failed_standalone_usage_write_rolls_back_and_can_be_retried_after_reopen() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let before = db.usage(S1).unwrap();
    ledger(&sb).execute_batch("CREATE TRIGGER fail_write BEFORE INSERT ON responses BEGIN SELECT RAISE(ABORT,'standalone failure'); END;").unwrap();
    let error = db.insert_usage(S1, "", &[row("standalone-failed")]).unwrap_err();
    assert!(format!("{error:#}").contains("standalone failure"));
    assert_eq!(db.usage(S1).unwrap(), before, "failed writes must not leave committed index rows");
    ledger(&sb).execute_batch("DROP TRIGGER fail_write;").unwrap();
    drop(db);
    let db = Db::open(&sb.db_path()).unwrap();
    assert_eq!(db.usage(S1).unwrap(), before);
    db.insert_usage(S1, "", &[row("standalone-failed")]).unwrap();
    assert_eq!(db.usage(S1).unwrap().len(), before.len() + 1);
    assert_eq!(ledger(&sb).query_row("SELECT count(*) FROM responses WHERE msg_id='standalone-failed'", [], |r| r.get::<_, i64>(0)).unwrap(), 1);
}

#[test]
fn standalone_project_and_usage_refresh_roll_back_when_the_ledger_fails() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let before = db.session(S1).unwrap().unwrap();
    ledger(&sb).execute_batch("CREATE TRIGGER fail_project BEFORE INSERT ON projects BEGIN SELECT RAISE(ABORT,'project failure'); END;").unwrap();
    let error = db.upsert_project(ALPHA, Some("/must-not-stick")).unwrap_err();
    assert!(format!("{error:#}").contains("project failure"));
    assert_eq!(db.session(S1).unwrap().unwrap().project_path, before.project_path);
    ledger(&sb).execute_batch("DROP TRIGGER fail_project; CREATE TRIGGER fail_session BEFORE INSERT ON sessions BEGIN SELECT RAISE(ABORT,'session failure'); END;").unwrap();
    db.conn().execute("UPDATE api_usage SET input=input+100 WHERE session_id=?1", [S1]).unwrap();
    let error = db.refresh_session_usage(S1).unwrap_err();
    assert!(format!("{error:#}").contains("session failure"));
    let after = db.session(S1).unwrap().unwrap();
    assert_eq!(after.tokens_in, before.tokens_in);
    assert_eq!(after.model_usage, before.model_usage);
}

#[test]
fn usage_accepts_more_ids_than_sqlite_bind_parameters_and_keeps_missing_metadata() {
    let l = Ledger::open_in_memory().unwrap();
    let index = Db::open_in_memory().unwrap();
    let mut batch = LedgerBatch::default();
    batch.responses.push(("included".into(), "".into(), row("a")));
    batch.responses.push(("excluded".into(), "".into(), row("b")));
    l.write(&batch, index.conn()).unwrap();
    let mut ids: Vec<_> = (0..40_000).map(|n| format!("absent-{n}")).collect();
    ids.extend(["included".into(), "included".into()]);
    let usage = l.usage(Some(&ids)).unwrap();
    assert_eq!(usage.len(), 1);
    assert_eq!(usage[0].session.id, "included");
    assert_eq!(usage[0].hours[0].responses, 1);
    assert!(l.usage(Some(&[])).unwrap().is_empty());
}

#[test]
fn a_read_only_backup_can_be_imported_without_opening_or_rebuilding_an_index() {
    let sb = sandbox();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let expected = db.ledger_usage(None).unwrap().unwrap();
    let calls = count(db.conn(), "api_usage");
    db.set_user_version_for_test(4);
    drop(db);
    let before = std::fs::read(sb.db_path()).unwrap();
    let source = Connection::open_with_flags(sb.db_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
    let target = sb.tmp.path().join("imported.sqlite");
    let ledger = Ledger::open(&target).unwrap();
    ledger.salvage_into_ledger(&source).unwrap();
    assert_eq!(ledger.usage(None).unwrap(), expected);
    ledger.salvage_into_ledger(&source).unwrap();
    assert_eq!(ledger.usage(None).unwrap(), expected);
    assert_eq!(ledger.response_counts().unwrap().values().sum::<u64>(), calls as u64);
    drop(source);
    assert_eq!(std::fs::read(sb.db_path()).unwrap(), before);
    assert_eq!(Connection::open_with_flags(sb.db_path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)
        .unwrap().query_row("PRAGMA user_version", [], |r| r.get::<_, i32>(0)).unwrap(), 4);
}

/// Supply snapshot paths in ASM_LEDGER_IMPORT_SOURCES (OS path-list syntax) and an explicit
/// ASM_LEDGER_IMPORT_TARGET. Sources are never opened through Db, which could rebuild them.
#[test]
#[ignore = "writes an explicitly supplied ledger; rehearse on snapshots before importing live data"]
fn import_usage_from_an_index_backup() {
    let sources = std::env::var_os("ASM_LEDGER_IMPORT_SOURCES").expect("source snapshot paths");
    let paths: Vec<_> = std::env::split_paths(&sources).collect();
    assert!(!paths.is_empty());
    let target = std::path::PathBuf::from(std::env::var_os("ASM_LEDGER_IMPORT_TARGET").expect("target ledger path"));
    // Resolve aliases before opening the destination for writing.
    let target_identity = target.canonicalize().unwrap_or_else(|_| {
        target.parent().unwrap().canonicalize().unwrap().join(target.file_name().unwrap())
    });
    let connections: Vec<_> = paths.iter().map(|path| {
        assert_ne!(path.canonicalize().unwrap(), target_identity, "source cannot be the target");
        Connection::open_with_flags(path, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap()
    }).collect();
    let ledger = Ledger::open(&target).unwrap();
    for (path, source) in paths.iter().zip(&connections) {
        let before = ledger.response_counts().unwrap();
        ledger.salvage_into_ledger(source).unwrap();
        let after = ledger.response_counts().unwrap();
        println!("source={} calls_before={} calls_after={} usage_sessions_before={} usage_sessions_after={}",
            path.display(), before.values().sum::<u64>(), after.values().sum::<u64>(), before.len(), after.len());
    }
    let mut usage = agent_session_core::usage::UsageData::default();
    usage.replace_all(ledger.usage(None).unwrap());
    let days: std::collections::BTreeSet<_> = usage.sessions.values().flat_map(|s| &s.hours)
        .map(|hour| agent_session_core::usage::usage_day(hour.hour * 3_600_000, &chrono::Local)).collect();
    for day in days {
        let start = agent_session_core::usage::day_start_ms(day, &chrono::Local);
        let end = agent_session_core::usage::day_start_ms(day.succ_opt().unwrap(), &chrono::Local);
        let totals = usage.totals(start, end);
        println!("day={day} calls={} cost={:.12} unpriced={}",
            totals.responses, totals.cost.total(), totals.unpriced_responses);
    }
}

#[test]
fn a_version_rebuild_corrects_iterations_in_the_ledger_but_keeps_deleted_sessions() {
    let sb = sandbox();
    let fixture = std::fs::read_to_string(fixture_root().parent().unwrap().join("iterations.jsonl")).unwrap();
    std::fs::write(sb.transcript(ALPHA, "iterations"), &fixture).unwrap();
    let mut db = Db::open(&sb.db_path()).unwrap();
    scan(&sb, &mut db);
    let deleted = db.ledger_usage(Some(&[S1.into()])).unwrap().unwrap();
    // Seed the exact v6 result: outer zeros, but the 5m/1h detail was already counted.
    let old = UsageRow { msg_id: "iteration-0".into(), model: "claude-sonnet-5".into(),
        ts: 1794823200000, input: 0, output: 0, cache_read: 0,
        cache_write_5m: 20, cache_write_1h: 80, web_search: 0, fast: false,
        cost_usd: Some(0.000370) };
    db.transaction(|db| {
        db.insert_usage("iterations", "", &[old])?;
        db.refresh_session_usage("iterations")
    }).unwrap();
    assert_eq!(ledger(&sb).query_row("SELECT cache_read FROM responses WHERE session_id='iterations' AND msg_id='iteration-0'", [], |r| r.get::<_,i64>(0)).unwrap(), 0);
    db.set_user_version_for_test(6);
    drop(db);
    remove_s1(&sb);
    let mut db = Db::open(&sb.db_path()).unwrap();
    assert_eq!(count(db.conn(), "sessions"), 0, "v6 must trigger a schema rebuild");
    assert_eq!(db.ledger_usage(Some(&[S1.into()])).unwrap().unwrap(), deleted);
    scan(&sb, &mut db);
    let corrected = ledger(&sb).query_row("SELECT input,output,cache_read,cache_write_5m,cache_write_1h FROM responses WHERE session_id='iterations' AND msg_id='iteration-0'", [],
        |r| Ok((r.get::<_,i64>(0)?,r.get::<_,i64>(1)?,r.get::<_,i64>(2)?,r.get::<_,i64>(3)?,r.get::<_,i64>(4)?))).unwrap();
    assert_eq!(corrected, (2,223,364422,20,80));
    let usage = db.ledger_usage(Some(&["iterations".into()])).unwrap().unwrap();
    let mut tokens = agent_session_core::model::Tokens::default();
    for h in &usage[0].hours { tokens.add(&h.tokens); }
    assert_eq!((tokens.input,tokens.output,tokens.cache_read,tokens.cache_write_5m,tokens.cache_write_1h,tokens.web_search), (203002,4723,376422,1220,380,2));
    assert_eq!(usage[0].hours.iter().map(|h| h.responses).sum::<u64>(), 2);
    assert_eq!(db.ledger_usage(Some(&[S1.into()])).unwrap().unwrap(), deleted);
}
