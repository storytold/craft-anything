// Original queue and filesystem fixtures by Afloat16, under the repository's
// MIT OR Apache-2.0 license. No external recordings, credentials, or model service.
use super::*;
use craft_anything_engine::workspace::WebsiteLanguage;

#[test]
fn a_failed_withdrawal_keeps_the_note_queued_until_it_can_be_removed() {
    let directory = tempfile::tempdir().unwrap();
    let ws = Workspace::create(directory.path(), "Notes", "Build an app", TargetRef::None).unwrap();
    let run = bare_run(&ws);
    let text = "Keep the original note and its picture";
    let pictures = vec![ws.root.join("reference.png")];
    let id = run.add_note(text.into(), text.into(), pictures.clone()).unwrap().id;
    let dir = notes::notes_dir(&ws.inbox_dir());
    let waiting = dir.join(format!("{id}.json"));
    let held = dir.join("waiting-note-held-by-test");
    let original = std::fs::read(&waiting).unwrap();
    std::fs::rename(&waiting, &held).unwrap();
    // A directory produces a real removal error even on a privileged test runner.
    std::fs::create_dir(&waiting).unwrap();

    assert_eq!(run.unsend(id).unwrap_err().code, ErrorCode::FileError);

    assert_eq!(run.waiting().iter().map(|note| note.id).collect::<Vec<_>>(), [id]);
    assert_eq!(run.waiting()[0].text, text);
    assert_eq!(run.waiting()[0].pictures, pictures);
    assert_eq!(std::fs::read(&held).unwrap(), original);
    std::fs::remove_dir(&waiting).unwrap();
    std::fs::rename(&held, &waiting).unwrap();
    let returned = run.unsend(id).unwrap().unwrap();
    assert_eq!(returned.id, id);
    assert_eq!(returned.text, text);
    assert_eq!(returned.pictures, pictures);
    assert!(run.waiting().is_empty());
    assert!(notes::claim(&dir).is_empty(), "a successful withdrawal cannot still reach the hook");
    assert!(notes::take_left(&dir).unwrap().is_empty());

    let read_id = run.add_note("Already delivered".into(), "Already delivered".into(), Vec::new()).unwrap().id;
    assert_eq!(notes::claim(&dir), ["Already delivered"]);
    assert!(run.unsend(read_id).unwrap().is_none(), "a claimed note cannot return as a draft");
    assert_eq!(run.waiting().iter().map(|note| note.id).collect::<Vec<_>>(), [read_id]);
    assert!(notes::was_read(&dir, read_id));
}

#[test]
fn a_hook_claim_behind_an_unreadable_note_is_not_repeated_in_the_followup() {
    let directory = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(directory.path(), "Site", "Build the website in English", TargetRef::None).unwrap();
    let run = bare_run(&ws);
    let first_text = "Translate the site into French";
    let second_text = "Translate the site into German";
    let first = run.add_note(first_text.into(), first_text.into(), Vec::new()).unwrap().id;
    let second = run.add_note(second_text.into(), second_text.into(), Vec::new()).unwrap().id;
    let dir = notes::notes_dir(&ws.inbox_dir());
    let first_path = dir.join(format!("{first}.json"));
    let held = dir.join("first-note-held-by-test");
    std::fs::rename(&first_path, &held).unwrap();
    std::fs::create_dir(&first_path).unwrap();
    let hook = notes::claim(&dir);
    assert_eq!(hook, [second_text], "the hook skips the unreadable older file and delivers the later note");
    let mut worker = notes_worker(&ws, &run);
    worker.self_check = Some((kind("website"), 90.0));
    let (tx, rx) = std::sync::mpsc::channel();
    worker.tx = tx;
    worker.relay_read_notes(&mut ws).unwrap();
    assert_eq!(run.waiting().len(), 2, "the live prefix relay still waits for the older observation");
    std::fs::remove_dir(&first_path).unwrap();
    std::fs::rename(&held, &first_path).unwrap();

    let next = worker.added(&mut ws, false, true).unwrap().unwrap();

    assert_eq!(next.note_ids, [first]);
    assert!(next.prompt.contains(first_text));
    assert!(!next.prompt.contains(second_text), "the actual hook output is not repeated in the next prompt");
    assert!(notes::claim(&dir).is_empty(), "the prompt now owns the remaining waiting note");
    assert_eq!(notes::take_left(&dir).unwrap().iter().map(|note| note.id).collect::<Vec<_>>(), [first]);
    assert!(run.waiting().is_empty());
    assert!(!notes::was_read(&dir, second), "the hook-delivered note has completed cleanup");
    assert_eq!(Workspace::open(&ws.root).unwrap().manifest.website_language, Some(WebsiteLanguage::Requested { tag: "de".into() }));
    let shown: Vec<_> = rx.try_iter().filter_map(|event| if let RunEvent::Note { shown, .. } = event { Some(shown) } else { None }).collect();
    assert_eq!(shown.iter().filter(|text| text.as_str() == first_text).count(), 1);
    assert_eq!(shown.iter().filter(|text| text.as_str() == second_text).count(), 1);
}

#[test]
fn a_partly_prepared_prompt_keeps_its_own_claims_for_retry() {
    let directory = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(directory.path(), "Notes", "Build an app", TargetRef::None).unwrap();
    let run = bare_run(&ws);
    let first_text = "Make the header blue";
    let second_text = "Add a footer";
    let first = run.add_note(first_text.into(), first_text.into(), Vec::new()).unwrap().id;
    let second = run.add_note(second_text.into(), second_text.into(), Vec::new()).unwrap().id;
    let dir = notes::notes_dir(&ws.inbox_dir());
    let second_path = dir.join(format!("{second}.json"));
    let original = std::fs::read(&second_path).unwrap();
    let mut replaced: Value = serde_json::from_slice(&original).unwrap();
    replaced["left"]["shown"] = json!("A changed second request");
    std::fs::write(&second_path, serde_json::to_vec(&replaced).unwrap()).unwrap();
    let mut worker = notes_worker(&ws, &run);

    assert!(worker.added(&mut ws, false, true).is_err());

    assert!(dir.join(format!("{first}.read.json")).is_file(), "the first prompt claim succeeded before the later identity refusal");
    assert!(!dir.join(format!("{first}.recorded.json")).exists(), "preparing a prompt has not delivered it");
    assert_eq!(run.waiting().iter().map(|note| note.id).collect::<Vec<_>>(), [first, second]);
    worker.relay_read_notes(&mut ws).unwrap();
    assert_eq!(run.waiting().iter().map(|note| note.id).collect::<Vec<_>>(), [first, second], "the worker must not mistake its own claim for hook delivery");
    std::fs::write(&second_path, original).unwrap();

    let next = worker.added(&mut ws, false, true).unwrap().unwrap();

    assert_eq!(next.note_ids, [first, second]);
    assert!(next.prompt.contains(first_text) && next.prompt.contains(second_text), "neither undelivered note can be lost on a same-worker retry");
    assert!(!dir.join(format!("{first}.recorded.json")).exists());
    assert!(!dir.join(format!("{second}.recorded.json")).exists());
    assert!(notes::claim(&dir).is_empty(), "the prepared prompt owns both claims");
    assert_eq!(
        notes::take_left(&dir).unwrap().iter().map(|note| note.id).collect::<Vec<_>>(),
        [first, second],
        "restart can recover both until delivery succeeds"
    );
}

#[test]
fn partial_completion_hands_only_unfinished_notes_to_the_next_run() {
    for fail_first in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(directory.path(), "Site", "Build the website in English", TargetRef::None).unwrap();
        let run = bare_run(&ws);
        let first_text = "Translate the site into French";
        let second_text = "Translate the site into German";
        let first = run.add_note(first_text.into(), first_text.into(), Vec::new()).unwrap().id;
        let second = run.add_note(second_text.into(), second_text.into(), Vec::new()).unwrap().id;
        let dir = notes::notes_dir(&ws.inbox_dir());
        assert_eq!(notes::claim(&dir), [first_text, second_text]);
        let blocker = dir.join(format!("{}.recorded.json", if fail_first { first } else { second }));
        std::fs::create_dir(&blocker).unwrap();
        let mut worker = notes_worker(&ws, &run);
        worker.self_check = Some((kind("website"), 90.0));
        let (tx, rx) = std::sync::mpsc::channel();
        worker.tx = tx;

        assert!(worker.relay_read_notes(&mut ws).is_err());

        let expected = if fail_first { vec![first, second] } else { vec![second] };
        let remaining = run.take_notes();
        assert_eq!(remaining.iter().map(|note| note.id).collect::<Vec<_>>(), expected);
        assert!(worker.reset_progress_after_notes, "instructions already delivered still reset the next attempt");
        assert_eq!(worker.taken_in_turn.iter().map(|note| note.id).collect::<Vec<_>>(), if fail_first { Vec::new() } else { vec![first] });
        let shown: Vec<_> = rx.try_iter().filter_map(|event| if let RunEvent::Note { shown, .. } = event { Some(shown) } else { None }).collect();
        assert_eq!(shown, if fail_first { Vec::new() } else { vec![first_text.to_string()] });
        std::fs::remove_dir(&blocker).unwrap();
        let mut opts = RunOptions {
            message: Some(remaining.iter().map(|note| note.text.as_str()).collect::<Vec<_>>().join("\n\n")),
            carried_note_ids: remaining.iter().map(|note| note.id).collect(),
            request_parts: remaining,
            ..RunOptions::default()
        };
        let left = notes::take_left(&dir).unwrap();
        prepend_older_notes(&mut opts, &left).unwrap();
        assert!(restore_left_notes(&ws, false, &mut opts, &left).unwrap().is_empty());
        assert_eq!(opts.carried_note_ids, expected);
        assert_eq!(opts.request_parts.iter().map(|note| note.id).collect::<Vec<_>>(), expected);
        assert_eq!(opts.message.as_deref().unwrap().contains(first_text), fail_first, "the UI handoff cannot recreate a completed request");
        assert!(opts.message.as_deref().unwrap().contains(second_text));
        assert!(notes::claim(&dir).is_empty(), "the recovered prompt owns its unfinished notes");
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.website_language, Some(WebsiteLanguage::Requested { tag: "de".into() }));
    }
}

#[test]
fn a_published_completion_does_not_reenter_handoff_when_its_sync_fails() {
    for (already_recorded, fail_at) in [(false, 1), (false, 2), (false, 3), (true, 1)] {
        let directory = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(directory.path(), "Site", "Build the website in English", TargetRef::None).unwrap();
        let run = bare_run(&ws);
        let text = "Translate the site into German";
        let id = run.add_note(text.into(), text.into(), Vec::new()).unwrap().id;
        let dir = notes::notes_dir(&ws.inbox_dir());
        assert_eq!(notes::claim(&dir), [text]);
        let mut worker = notes_worker(&ws, &run);
        worker.self_check = Some((kind("website"), 90.0));
        let read = run.take_notes();
        worker.record_request_texts(&mut ws, &read).unwrap();
        if already_recorded {
            notes::mark_recorded(&dir, id).unwrap();
        }
        let calls = std::cell::Cell::new(0);

        let error = worker
            .complete_read_notes_with_sync(&ws, read, |path| {
                calls.set(calls.get() + 1);
                if calls.get() == fail_at {
                    if already_recorded || fail_at == 3 {
                        assert!(path.join(format!("{id}.recorded.json")).is_file(), "the failed sync follows a real completion publication");
                    }
                    Err(ToolError::io(path.display(), std::io::Error::other("injected completion directory sync failure")))
                } else {
                    notes::sync_directory(path)
                }
            })
            .unwrap_err();

        assert!(error.message.contains("injected completion directory sync failure"), "the persistence failure still reaches the caller");
        let handed = run.take_notes();
        let published = already_recorded || fail_at == 3;
        assert_eq!(handed.iter().map(|note| note.id).collect::<Vec<_>>(), if published { Vec::new() } else { vec![id] });
        assert_eq!(notes::take_left(&dir).unwrap().is_empty(), published);
        assert_eq!(worker.taken_in_turn.iter().map(|note| note.id).collect::<Vec<_>>(), if published { vec![id] } else { Vec::new() });
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.website_language, Some(WebsiteLanguage::Requested { tag: "de".into() }));
    }
}

struct CheckCompletedBuild(Arc<std::sync::atomic::AtomicUsize>);

impl Check for CheckCompletedBuild {
    fn id(&self) -> &'static str {
        "completed-build"
    }

    fn run(&self, cx: &mut CheckCx) -> Result<CheckResult, ToolError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Ok(CheckResult::new(self.id(), cx.round, true, "The completed build passed its check."))
    }
}

struct CompletedBuildKind(Arc<std::sync::atomic::AtomicUsize>);

impl TargetKind for CompletedBuildKind {
    fn id(&self) -> &'static str {
        "completed-build"
    }

    fn words(&self) -> KindWords {
        kind("app").words()
    }

    fn detect(&self, _: &DetectCx) -> Option<u32> {
        Some(2000)
    }

    fn default_stack(&self, _: &DetectCx) -> &'static str {
        "Anything"
    }

    fn goal(&self) -> &'static str {
        "an app"
    }

    fn checks(&self, _: &Workspace) -> Vec<Box<dyn Check>> {
        vec![Box::new(CheckCompletedBuild(self.0.clone()))]
    }
}

#[test]
fn completed_note_cleanup_failure_keeps_check_only_recovery() {
    use crate::agent::{ApiConversation, EngineHost, LoopConfig, ModelSlot};
    use crate::model::{ChatModel, Delta, Stop, TurnRequest, TurnResult};
    use std::sync::atomic::AtomicUsize;

    struct DoNotBuildAgain(Arc<AtomicUsize>);
    impl ChatModel for DoNotBuildAgain {
        fn turn(&mut self, _: &TurnRequest<'_>, _: &mut dyn FnMut(Delta), _: &Stop) -> Result<TurnResult, ToolError> {
            self.0.fetch_add(1, Ordering::SeqCst);
            Err(ToolError::internal("A completed build must resume with checks, not another builder request."))
        }

        fn model(&self) -> String {
            "queue-fixture".into()
        }
    }

    let directory = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(directory.path(), "Completed build", "Build an app", TargetRef::None).unwrap();
    ws.manifest.stack = "Anything".into();
    ws.manifest.capture_deep = Some(false);
    ws.manifest.last_reply = "An earlier reply".into();
    ws.save().unwrap();
    std::fs::create_dir_all(ws.capture_dir()).unwrap();
    std::fs::create_dir_all(ws.app_dir()).unwrap();
    std::fs::write(ws.app_dir().join("built.txt"), "The finished build").unwrap();
    let dir = notes::notes_dir(&ws.inbox_dir());
    let left = notes::Left { text: "Use larger text".into(), shown: "Use larger text".into(), pictures: Vec::new(), added: None };
    notes::hand_over(&dir, 71, &left.text, Some(&left)).unwrap();
    assert!(notes::carry_for_prompt(&dir, 71, &left, || Ok(())).unwrap());
    let error = save_completed_turn(&mut ws, "Finished the requested edit", &mut vec![71], |dir, id| {
        notes::forget_using(dir, id, |path| {
            assert!(!path.join("71.read.json").exists());
            assert!(!path.join("71.recorded.json").exists(), "the failure follows actual marker removal");
            Err(ToolError::io(path.display(), std::io::Error::other("injected final note-directory sync failure")))
        })
    })
    .unwrap_err();
    assert!(error.message.contains("injected final note-directory sync failure"));
    let mut saved = Workspace::open(&ws.root).unwrap();
    assert!(saved.manifest.awaiting_check, "cleanup cannot erase the successful turn checkpoint");
    assert_eq!(saved.manifest.last_reply, "Finished the requested edit");
    assert!(notes::take_left(&dir).unwrap().is_empty());

    let mut worker = check_worker(&saved, false);
    let calls = Arc::new(AtomicUsize::new(0));
    let checks = Arc::new(AtomicUsize::new(0));
    let slot = ModelSlot {
        name: "Queue fixture".into(),
        model: Box::new(DoNotBuildAgain(calls.clone())),
        local: true,
        effort: None,
        max_turns: 10,
        context_tokens: 0,
        tools: Default::default(),
    };
    let api = ApiConversation::start(Arc::new(EngineHost(worker.engine.clone())), vec![slot], LoopConfig::default()).unwrap();
    let entry = crate::BackendEntry {
        config: crate::BackendConfig {
            name: "Queue fixture".into(),
            kind: BackendKind::OpenaiCompatible,
            model: "queue-fixture".into(),
            base_url: None,
            ssh_host: None,
            enabled: true,
        },
        options: Default::default(),
    };
    let mut opts = RunOptions { continuing: true, ..RunOptions::default() };
    opts.kinds.register(Arc::new(CompletedBuildKind(checks.clone())));

    worker.run(&mut saved, entry, Some(api), None, KeptHistory { in_mode: true, elsewhere: false }, &opts).unwrap();

    assert_eq!(calls.load(Ordering::SeqCst), 0, "the real message-free worker never asks its builder again");
    assert_eq!(checks.load(Ordering::SeqCst), 1);
    let saved = Workspace::open(&saved.root).unwrap();
    assert!(!saved.manifest.awaiting_check);
    assert_eq!(saved.manifest.last_reply, "Finished the requested edit");
}
