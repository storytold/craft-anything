//! A runner's build with nobody watching (#1970): it pauses before its next builder turn once
//! nobody has watched for its limit, unless the person chose to keep building, and says what
//! happened with a notification while nobody watches. The builder changes a file every turn and
//! the check never passes, so a build that isn't stopped takes all its rounds.

use super::stall::run_busy_builder_with;
use super::*;
use crate::rebuild::unwatched::{Notify, UNWATCHED_PAUSE, Watch, WhenUnwatched, keep_path};
use craft_anything_engine::workspace::watchers::{self, Watching};

/// The notifications a test's runs sent, as (title, body).
type Told = Arc<Mutex<Vec<(String, String)>>>;

fn project() -> (tempfile::TempDir, Workspace) {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(dir.path(), "Notes", "Build an app", TargetRef::None).unwrap();
    ws.manifest.stack = "Scripted stack".into();
    ws.save().unwrap();
    (dir, ws)
}

/// A runner's rule with `limit`, whose notifications are kept instead of shown.
fn rule(limit: Duration) -> (WhenUnwatched, Told) {
    let told: Told = Arc::default();
    let t = told.clone();
    let notify = Notify::new(move |title, body| t.lock().unwrap().push((title.to_string(), body.to_string())));
    (WhenUnwatched { limit, notify }, told)
}

/// One run, watched as `watch` says; what it ended with, its builder turns, and whether it paused
/// for nobody.
fn run(ws: &mut Workspace, watch: Option<Arc<Watch>>) -> (ToolError, usize, bool) {
    let seen = Arc::new(Mutex::new(0));
    let (error, worker) = run_busy_builder_with(ws, &seen, Some("Build the app"), |w| w.watch = watch);
    let turns = *seen.lock().unwrap();
    (error, turns, worker.paused_for_nobody)
}

#[test]
fn a_watched_runner_build_goes_on() {
    let (_d, mut ws) = project();
    let follower = Watching::begin(&ws.root).unwrap().unwrap();
    let (rule, _) = rule(Duration::ZERO);
    let (error, turns, for_nobody) = {
        let w = Watch::new(&ws, rule);
        run(&mut ws, Some(w))
    };
    assert!(error.message.contains("isn't working yet"), "it took all its rounds: {error:?}");
    assert_eq!(turns, 7, "the first turn and six refinements");
    assert!(!for_nobody);
    drop(follower);
}

#[test]
fn an_unwatched_runner_build_pauses_before_its_next_turn() {
    let (_d, mut ws) = project();
    assert!(!watchers::watched(&ws.root));
    let (rule, _) = rule(Duration::ZERO);
    let (error, turns, for_nobody) = {
        let w = Watch::new(&ws, rule);
        run(&mut ws, Some(w))
    };
    assert_eq!(error.code, ErrorCode::Cancelled, "{error:?}");
    assert!(for_nobody, "and it says why");
    assert_eq!(turns, 1, "never before its first turn, and not in the middle of one");
    assert_eq!(ws.manifest.checks.len(), 1, "the turn's check finished first");

    // What it says and saves, as an ordinary Pause does, with the reason.
    let (ok, message, paused) = super::super::run_outcome(Ok(Err(error)), &mut ws, for_nobody);
    assert_eq!((ok, message.as_str(), paused), (false, UNWATCHED_PAUSE, true));
    assert_eq!(Workspace::open(&ws.root).unwrap().manifest.status, Status::Stopped);
}

#[test]
fn a_build_unwatched_for_less_than_its_limit_goes_on() {
    let (_d, mut ws) = project();
    let (rule, _) = rule(Duration::from_secs(3600));
    let (error, turns, _) = {
        let w = Watch::new(&ws, rule);
        run(&mut ws, Some(w))
    };
    assert!(error.message.contains("isn't working yet"), "{error:?}");
    assert_eq!(turns, 7);
}

#[test]
fn keep_building_lets_an_unwatched_build_go_on() {
    let (_d, mut ws) = project();
    craft_anything_platform::fs::write_atomic(&keep_path(&ws.root), b"").unwrap();
    let (rule, _) = rule(Duration::ZERO);
    let (error, turns, for_nobody) = {
        let w = Watch::new(&ws, rule);
        run(&mut ws, Some(w))
    };
    assert!(error.message.contains("isn't working yet"), "{error:?}");
    assert_eq!(turns, 7);
    assert!(!for_nobody);
}

#[test]
fn a_build_that_isnt_a_runners_never_pauses_for_nobody() {
    let (_d, mut ws) = project();
    assert!(!watchers::watched(&ws.root));
    let (error, turns, for_nobody) = run(&mut ws, None);
    assert!(error.message.contains("isn't working yet"), "{error:?}");
    assert_eq!((turns, for_nobody), (7, false));
}

#[test]
fn a_window_that_follows_watches_and_takes_keep_building_back() {
    let (_d, ws) = project();
    let owner = Elsewhere::start(&ws, "Make it blue").unwrap();
    craft_anything_platform::fs::write_atomic(&keep_path(&ws.root), b"").unwrap();
    let (rule, _) = rule(Duration::ZERO);
    let watch = Watch::new(&ws, rule);
    assert!(!watch.should_pause(), "kept going while the marker is there");

    let run = RebuildRun::attach(&ws, Wake::default()).unwrap();
    assert!(!keep_path(&ws.root).exists(), "a window that follows takes the choice back");
    assert!(watchers::watched(&ws.root), "and watches");
    watch.look();
    assert!(!watch.should_pause());

    // The window goes (closed, or its process ended) without choosing: nobody watches, and the
    // build pauses at its next turn.
    drop(run);
    assert!(!watchers::watched(&ws.root), "released as the follower goes");
    watch.look();
    assert!(watch.should_pause());

    // Keep building, said for a followed run, is the marker again.
    let run = RebuildRun::attach(&ws, Wake::default()).unwrap();
    run.keep_building();
    drop(run);
    watch.look();
    assert!(keep_path(&ws.root).exists() && !watch.should_pause());
    drop(owner);
}

#[test]
fn nobody_watching_is_told_once_when_it_ends_or_asks() {
    let (_d, ws) = project();
    let inbox = Inbox::new(ws.inbox_dir());
    let id = inbox.post(craft_anything_engine::inbox::RequestKind::Question, "Blue or teal?", "", vec![], true, None).unwrap();
    let question = inbox.pending().into_iter().find(|r| r.id == id).unwrap();
    let wait_for = |told: &Told, n: usize| {
        let began = Instant::now();
        while told.lock().unwrap().len() < n && began.elapsed() < Duration::from_secs(10) {
            std::thread::sleep(Duration::from_millis(10));
        }
    };

    // Watched: never.
    let follower = Watching::begin(&ws.root).unwrap().unwrap();
    let (r, told) = rule(Duration::ZERO);
    let watch = Watch::new(&ws, r);
    watch.asking(&question);
    watch.finished(true, "Ready.", false, false);
    watch.finished(false, "It broke.", false, false);
    std::thread::sleep(Duration::from_millis(100));
    assert!(told.lock().unwrap().is_empty(), "{:?}", told.lock().unwrap());
    drop(follower);

    // Nobody: once for each question, and once for the end.
    watch.look();
    watch.asking(&question);
    watch.asking(&question);
    wait_for(&told, 1);
    std::thread::sleep(Duration::from_millis(100));
    assert_eq!(told.lock().unwrap().clone(), [("Craft Anything".to_string(), "Notes: the build needs your answer.".to_string())]);
    let ends = [
        ((true, "Ready: looks 97% alike.", false, false), Some("Notes: the build is ready.")),
        ((false, "The build isn't working yet.\nAsk for a fix.", false, false), Some("Notes: the build stopped: The build isn't working yet.")),
        ((false, UNWATCHED_PAUSE, true, true), Some("Notes: the build paused.")),
        // A Pause someone asked for isn't news to them.
        ((false, "Paused.", true, false), None),
    ];
    for ((ok, message, paused, for_nobody), said) in ends {
        told.lock().unwrap().clear();
        watch.finished(ok, message, paused, for_nobody);
        let got: Vec<String> = told.lock().unwrap().iter().map(|(_, b)| b.clone()).collect();
        assert_eq!(got, said.into_iter().map(String::from).collect::<Vec<_>>(), "{message}");
    }
}
