//! A run followed from another process. The lock is per open file, so a follower in the same test
//! process sees the owner's lock as a build elsewhere would; the owner is made of the pieces
//! [`RebuildRun::start`] uses (its lock, event log, feed and control), driven by hand.

use super::*;
use craft_anything_engine::feed::PlanStatus;
use craft_anything_engine::inbox::RequestKind;

/// A run as `RebuildRun::start` sets it up, without a worker: the test says what it does.
type Owner = Elsewhere;

fn project() -> (tempfile::TempDir, Workspace) {
    let d = tempfile::tempdir().unwrap();
    let ws = Workspace::create(d.path(), "Notes", "A notes app", TargetRef::None).unwrap();
    (d, ws)
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    let began = Instant::now();
    while !done() {
        assert!(began.elapsed() < Duration::from_secs(15), "never: {what}");
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Collect a follower's events until it finishes.
fn follow_to_the_end(run: &mut RebuildRun) -> Vec<RunEvent> {
    let mut seen = Vec::new();
    wait_for("the follower finishes", || {
        seen.extend(run.poll());
        !run.is_running()
    });
    seen.extend(run.poll());
    seen
}

fn checklist(status: &str) -> RunEvent {
    RunEvent::Agent(AgentEvent::ToolStarted {
        call_id: "t".into(),
        tool: "TodoWrite".into(),
        input: json!({"todos": [{"content": "Lay out the page", "activeForm": "Laying out the page", "status": status}]}),
    })
}

fn texts(lines: Vec<activity::ActivityLine>) -> Vec<String> {
    lines.into_iter().map(|l| l.text).collect()
}

#[test]
fn a_follower_sees_every_event_and_ends_with_the_runs_feed() {
    let (_d, ws) = project();
    // An earlier run left a checklist still doing a step: this run updates it in place, which
    // changes an item from before it started.
    let earlier = vec![
        feed::FeedItem::User(feed::UserItem::Text("A notes app".into())),
        feed::FeedItem::Plan(vec![feed::PlanStep { text: "Lay out the page".into(), doing: "Laying out".into(), status: PlanStatus::Doing }]),
        feed::FeedItem::Agent { text: "Started.".into(), streaming: false },
    ];
    feed::save(&ws.root, &earlier).unwrap();
    let mut owner = Owner::start(&ws, "Make it blue").unwrap();
    let woke = Arc::new(AtomicU64::new(0));
    let w = woke.clone();
    let mut run = RebuildRun::attach(
        &ws,
        Wake::new(move || {
            w.fetch_add(1, Ordering::SeqCst);
        }),
    )
    .unwrap();
    assert!(run.is_attached() && run.is_running());
    assert_eq!(run.feed_now(), owner.items(), "the feed as the run began");

    let inbox = Inbox::new(ws.inbox_dir());
    let id = inbox.post(RequestKind::Question, "Blue or teal?", "", vec![], true, None).unwrap();
    let question = inbox.pending().into_iter().find(|r| r.id == id).unwrap();
    for e in [
        RunEvent::Step { step: Step::Build, text: "Building".into() },
        checklist("in_progress"),
        RunEvent::Agent(AgentEvent::TextDelta { text: "Working on it.\nNow the ".into() }),
        RunEvent::Agent(AgentEvent::ToolStarted { call_id: "b".into(), tool: "Bash".into(), input: json!({"command": "npm run build"}) }),
        RunEvent::Agent(AgentEvent::ToolFinished { call_id: "b".into(), tool: "Bash".into(), ok: true, summary: String::new(), evidence_id: None }),
        RunEvent::Ask(question),
        RunEvent::Preview("http://127.0.0.1:1234/".into()),
        checklist("completed"),
    ] {
        owner.emit(e);
    }
    // Answered in the run's own process: the follower's feed says so too.
    owner.feed.lock().unwrap().answered(&id, "Teal");
    owner.finish(true, "Ready.", false);

    let seen = follow_to_the_end(&mut run);
    assert_eq!(seen, owner.events(), "the same events, in order");
    assert_eq!(run.feed_now(), owner.items(), "the same feed");
    assert!(run.feed_now().iter().any(|i| matches!(i, feed::FeedItem::Ask { answer: Some(a), .. } if a == "Teal")));
    assert_eq!(run.feed_now().iter().filter(|i| matches!(i, feed::FeedItem::Plan(_))).count(), 1, "the earlier checklist was updated in place");
    assert_eq!(run.preview_url().as_deref(), Some("http://127.0.0.1:1234/"));
    let mine = owner.feed.lock().unwrap().activity();
    assert_eq!(texts(run.activity_lines(0..run.activity_len())), texts(mine.lines_in(0..mine.len())), "the same activity");
    assert!(woke.load(Ordering::SeqCst) > 0, "the window was woken");
    assert!(!run.is_running());
}

#[test]
fn a_follower_that_comes_in_part_way_has_the_whole_feed_so_far() {
    let (_d, ws) = project();
    let mut owner = Owner::start(&ws, "Make it blue").unwrap();
    owner.emit(RunEvent::Step { step: Step::Build, text: "Building".into() });
    owner.emit(checklist("in_progress"));
    owner.emit(RunEvent::Agent(AgentEvent::TextDelta { text: "Half".into() }));
    owner.emit(RunEvent::Agent(AgentEvent::ToolStarted { call_id: "r".into(), tool: "Read".into(), input: json!({"file_path": "src/main.rs"}) }));
    // Longer than the 30 ms looked for below: the log keeps whole milliseconds (the run's start,
    // each event's time and the follower's clock), so a follower can count a few less than ran.
    std::thread::sleep(Duration::from_millis(40));

    let mut run = RebuildRun::attach(&ws, Wake::default()).unwrap();
    assert_eq!(run.feed_now(), owner.items(), "everything so far, before the first poll");
    let now = run.activity_now();
    assert_eq!(now.current.as_ref().map(|(a, _)| a.words()), Some("Reading src/main.rs".into()), "{now:?}");
    assert!(now.current.is_some_and(|(_, d)| d >= Duration::from_millis(30)), "timed from when the run started it");
    assert!(run.started() <= Instant::now());

    owner.emit(RunEvent::Agent(AgentEvent::TextDelta { text: " done.".into() }));
    wait_for("the next event", || run.feed_now() == owner.items());
    owner.finish(true, "Ready.", false);
    follow_to_the_end(&mut run);
    assert_eq!(run.feed_now(), owner.items());
}

#[test]
fn stopping_a_follower_pauses_the_run() {
    let (_d, ws) = project();
    // A stop left from an earlier run's follower isn't this run's.
    craft_anything_platform::fs::write_atomic(&stop_path(&ws.root), b"").unwrap();
    let owner = Owner::start(&ws, "Make it blue").unwrap();
    assert!(!stop_path(&ws.root).exists(), "cleared when the run began");
    watch(owner.root.clone(), owner.stop.clone(), owner.notes.clone(), owner.feed.clone(), owner.running.clone(), Control::default());
    let mut run = RebuildRun::attach(&ws, Wake::default()).unwrap();

    // Dropping a follower leaves the build alone.
    drop(RebuildRun::attach(&ws, Wake::default()).unwrap());
    std::thread::sleep(CONTROL_EVERY * 2);
    assert!(!owner.stop.is_cancelled() && !stop_path(&ws.root).exists(), "dropping a follower stopped the build");

    run.stop();
    wait_for("the run is asked to stop", || owner.stop.is_cancelled());
    wait_for("the request is taken", || !stop_path(&ws.root).exists());
    let mut owner = owner;
    owner.finish(true, "Paused.", true);
    let seen = follow_to_the_end(&mut run);
    assert_eq!(seen.last(), Some(&RunEvent::Finished { ok: true, message: "Paused.".into(), paused: true }));
    assert!(matches!(run.feed_now().last(), Some(feed::FeedItem::Paused)));
}

#[test]
fn a_note_from_a_follower_waits_for_the_runs_builder_and_can_be_taken_back() {
    let (_d, ws) = project();
    let mut owner = Owner::start(&ws, "Make it blue").unwrap();
    let run = RebuildRun::attach(&ws, Wake::default()).unwrap();
    let dir = notes::notes_dir(&ws.inbox_dir());

    let id = run.add_note("make the header teal".into(), "make the header teal".into(), Vec::new());
    assert!(notes::is_foreign(id), "a follower's id can't be one the run hands out");
    assert_eq!(run.waiting().len(), 1, "it waits at once");
    owner.look();
    let waiting = owner.notes.lock().unwrap().clone();
    assert!(matches!(waiting.as_slice(), [n] if n.id == id && n.text == "make the header teal"), "{waiting:?}");
    owner.look();
    assert_eq!(owner.notes.lock().unwrap().len(), 1, "taken once");

    // Taken back before the builder read it: it goes from the run's notes too.
    assert_eq!(run.unsend(id).map(|n| n.text), Some("make the header teal".into()));
    assert!(run.waiting().is_empty() && run.queued() == 0);
    owner.look();
    assert!(owner.notes.lock().unwrap().is_empty(), "taken back from the run");
    assert!(!notes::take_for_builder(&dir, id), "and the builder can't have it");

    // One the builder read can't be taken back.
    let read = run.add_note("add a search box".into(), "add a search box".into(), Vec::new());
    assert_ne!(read, id);
    owner.look();
    assert_eq!(notes::claim(&dir), ["add a search box"]);
    assert!(run.unsend(read).is_none(), "the builder has it");
    owner.look();
    assert_eq!(owner.notes.lock().unwrap().len(), 1, "a note the builder read stays the run's to show");

    // The run ends with a note waiting: the follower can take it for the next run.
    let left = run.add_note("and a dark mode".into(), "and a dark mode".into(), Vec::new());
    wait_for("the follower sees it waiting", || run.waiting().iter().any(|n| n.id == left));
    owner.finish(true, "Ready.", false);
    assert_eq!(run.take_notes().into_iter().map(|n| n.id).collect::<Vec<_>>(), [left]);
    assert!(!notes::is_waiting(&dir, left), "taken, so no other run sends it too");
}

#[test]
fn an_answer_given_while_following_is_recorded_by_the_run() {
    let (_d, ws) = project();
    let mut owner = Owner::start(&ws, "Make it blue").unwrap();
    let inbox = Inbox::new(ws.inbox_dir());
    let id = inbox.post(RequestKind::Question, "Blue or teal?", "", vec![], true, None).unwrap();
    owner.emit(RunEvent::Ask(inbox.pending().into_iter().find(|r| r.id == id).unwrap()));
    let mut run = RebuildRun::attach(&ws, Wake::default()).unwrap();

    run.answer_as(&id, &Answer { choice: Some("Teal".into()), ..Answer::default() }, "Teal, please").unwrap();
    let answered = |items: &[feed::FeedItem]| items.iter().any(|i| matches!(i, feed::FeedItem::Ask { answer: Some(a), .. } if a == "Teal, please"));
    assert!(!answered(&feed::load(&ws)), "the follower didn't write the feed");
    owner.look();
    assert!(answered(&owner.items()), "the run recorded it");
    assert!(answered(&feed::load(&ws)), "and saved it");
    wait_for("the follower has the same feed", || run.feed_now() == owner.items());
    let got = inbox.wait(&id, Duration::from_millis(1), &|| false);
    assert_eq!(got.and_then(|a| a.choice).as_deref(), Some("Teal"), "the builder has its answer");
    owner.finish(true, "Ready.", false);
    follow_to_the_end(&mut run);
}

#[test]
fn a_run_whose_program_ended_without_a_word_ends_its_follower() {
    let (_d, ws) = project();
    // A run that never says it finished: its program was killed.
    let lock = BuildLock::take(&ws).unwrap().unwrap();
    let mut events = EventLog::begin(&ws.root);
    events.start(false, Duration::ZERO, &[feed::FeedItem::User(feed::UserItem::Text("A notes app".into()))]);
    events.event(Duration::from_millis(5), &RunEvent::Step { step: Step::Build, text: "Building".into() });
    let mut run = RebuildRun::attach(&ws, Wake::default()).unwrap();
    std::thread::sleep(FOLLOW_EVERY * 3);
    assert!(run.is_running(), "working while its lock is held");
    drop(lock);
    let seen = follow_to_the_end(&mut run);
    let ended = RunEvent::Finished { ok: false, message: ENDED_ELSEWHERE.into(), paused: false };
    assert_eq!(seen.iter().filter(|e| **e == ended).count(), 1, "said once: {seen:?}");
    assert_eq!(seen.last(), Some(&ended));
    assert!(matches!(run.feed_now().last(), Some(feed::FeedItem::Error { message, .. }) if message == ENDED_ELSEWHERE));
}

#[test]
fn a_new_run_replacing_the_log_ends_the_follower() {
    let (_d, ws) = project();
    let lock = BuildLock::take(&ws).unwrap().unwrap();
    let mut events = EventLog::begin(&ws.root);
    events.start(false, Duration::ZERO, &[]);
    let mut run = RebuildRun::attach(&ws, Wake::default()).unwrap();
    // Another run's log, while a lock is still held.
    let mut next = EventLog::begin(&ws.root);
    next.start(false, Duration::ZERO, &[]);
    next.event(Duration::ZERO, &RunEvent::Step { step: Step::Build, text: "Someone else's".into() });
    let seen = follow_to_the_end(&mut run);
    assert!(!seen.iter().any(|e| matches!(e, RunEvent::Step { .. })), "the new run's events aren't this one's: {seen:?}");
    assert!(matches!(seen.last(), Some(RunEvent::Finished { ok: false, .. })));
    drop(lock);
}

#[test]
fn nothing_to_follow_without_a_build_or_from_a_newer_version() {
    let (_d, ws) = project();
    let err = RebuildRun::attach(&ws, Wake::default()).err().unwrap();
    assert_eq!((err.code, err.message.as_str()), (ErrorCode::NotFound, NOT_BUILDING));
    assert!(err.remediation.is_some());

    // A log nobody holds the lock for is an old one.
    let mut events = EventLog::begin(&ws.root);
    events.start(false, Duration::ZERO, &[]);
    assert_eq!(RebuildRun::attach(&ws, Wake::default()).err().unwrap().code, ErrorCode::NotFound);

    let _lock = BuildLock::take(&ws).unwrap().unwrap();
    let newer = format!(
        "{}\n{}\n",
        json!({"header": {"version": EVENTS_VERSION + 1, "run": "r", "clean_room": false, "started_ms": 0, "more": true}}),
        json!({"base": {"items": []}})
    );
    std::fs::write(events_path(&ws.root), newer).unwrap();
    let err = RebuildRun::attach(&ws, Wake::default()).err().unwrap();
    assert_eq!(err.code, ErrorCode::NotSupported, "{err:?}");
    assert!(err.message.contains("newer"), "{}", err.message);
}

#[test]
fn a_log_line_written_in_parts_is_read_once_whole() {
    let (_d, ws) = project();
    std::fs::create_dir_all(ws.inbox_dir()).unwrap();
    let mut file = std::fs::File::create(events_path(&ws.root)).unwrap();
    let mut reader = Reader::open(&ws.root).unwrap();
    file.write_all(b"{\"answered\":{\"id\":\"a\",").unwrap();
    assert!(reader.lines().ok().unwrap().is_empty(), "half a line isn't a line");
    file.write_all(b"\"shown\":\"Yes\"}}\n{\"ans").unwrap();
    let lines = reader.lines().ok().unwrap();
    assert_eq!(lines, [b"{\"answered\":{\"id\":\"a\",\"shown\":\"Yes\"}}\n".to_vec()]);
    file.set_len(4).unwrap();
    assert!(reader.lines().is_err(), "a shorter file is a new one");
}

#[test]
fn what_looks_like_a_secret_is_not_logged() {
    let (_d, ws) = project();
    let _lock = BuildLock::take(&ws).unwrap().unwrap();
    let mut events = EventLog::begin(&ws.root);
    events.start(false, Duration::ZERO, &[]);
    events.event(
        Duration::ZERO,
        &RunEvent::Agent(AgentEvent::ToolStarted {
            call_id: "b".into(),
            tool: "Bash".into(),
            input: json!({"command": "curl -H 'Authorization: Bearer sk-test-0123456789abcdefghijklmnop' x", "api_key": "k"}),
        }),
    );
    let log = std::fs::read_to_string(events_path(&ws.root)).unwrap();
    assert!(!log.contains("sk-test-0123456789") && !log.contains("\"k\""), "{log}");
    assert!(log.contains("curl"), "{log}");
}

#[test]
fn a_set_follows_a_build_elsewhere_and_can_leave_it_working() {
    let (_d, ws) = project();
    let owner = Owner::start(&ws, "Make it blue").unwrap();
    let mut set = RunSet::new(1);
    let follow = || RebuildRun::attach(&ws, Wake::default()).unwrap();
    set.adopt(follow()).unwrap();
    assert!(set.is_running(&ws.root) && set.is_busy(&ws.root) && set.active() == 1 && set.followed() == 1);
    assert!(set.get(&ws.root).is_some_and(RebuildRun::is_attached));
    assert!(set.adopt(follow()).is_err(), "followed once");
    std::thread::sleep(CONTROL_EVERY * 2);
    assert!(set.is_running(&ws.root) && !stop_path(&ws.root).exists(), "the refused follower's drop left it working");
    set.detach_all();
    assert!(set.get(&ws.root).is_none() && !set.any_busy() && set.followed() == 0);
    assert!(!stop_path(&ws.root).exists() && !owner.stop.is_cancelled(), "left working");

    // stop_all pauses followed runs too.
    set.adopt(follow()).unwrap();
    set.stop_all();
    assert!(stop_path(&ws.root).exists());
}

/// A follower made on a worker isn't taken over for a project this set has queued.
#[test]
fn a_set_does_not_adopt_a_project_it_has_queued() {
    let d = tempfile::tempdir().unwrap();
    let a = Workspace::create(d.path(), "A", "A notes app", TargetRef::None).unwrap();
    let b = Workspace::create(d.path(), "B", "A notes app", TargetRef::None).unwrap();
    let mut s = craft_anything_engine::testing::session();
    s.settings.ai = toml::from_str("[[backends]]\nname = \"Agent\"\nkind = \"agent_cli\"\nmodel = \"claude-code\"\n").unwrap();
    let hub = AiHub::new(Arc::new(Engine::new(s)));
    let fake = StandIns::new();
    let mut set = RunSet::new(1).with_starter(fake.starter());
    set.start(&hub, a.clone(), RunOptions::default()).unwrap();
    assert_eq!(set.start(&hub, b.clone(), RunOptions::default()).unwrap(), Started::Queued { position: 1 });
    let _owner = Owner::start(&b, "Make it blue").unwrap();
    assert!(set.adopt(RebuildRun::attach(&b, Wake::default()).unwrap()).is_err(), "it's queued here");
    assert_eq!(set.position(&b.root), Some(1));
    set.stop_all();
}
