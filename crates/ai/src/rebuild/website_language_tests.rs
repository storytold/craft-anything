use super::*;
use craft_anything_engine::workspace::WebsiteLanguage;

fn note(id: u64, text: &str) -> Note {
    Note { id, text: text.into(), shown: text.into(), pictures: Vec::new(), questions: Vec::new(), added: std::time::SystemTime::now() }
}

#[test]
fn website_language_follows_delivered_notes_and_survives_restart() {
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(dir.path(), "Site", "Rebuild this website in French", TargetRef::None).unwrap();
    let website = kind("website");
    website.record_requests(&mut ws, &[]).unwrap();
    let mut worker = check_worker(&ws, false);
    worker.self_check = Some((website.clone(), 90.0));
    let inbox = notes::notes_dir(&ws.inbox_dir());
    let german = "Set the page language to de";
    worker.notes.lock().unwrap().push(note(71, german));
    notes::hand_over(&inbox, 71, german, None).unwrap();
    worker.relay_read_notes(&mut ws).unwrap();
    assert_eq!(ws.manifest.website_language, Some(WebsiteLanguage::Requested { tag: "fr".into() }), "queued notes are still retractable");
    assert_eq!(notes::claim(&inbox), [german]);
    worker.relay_read_notes(&mut ws).unwrap();
    assert_eq!(Workspace::open(&ws.root).unwrap().manifest.website_language, Some(WebsiteLanguage::Requested { tag: "de".into() }));
    assert!(worker.notes.lock().unwrap().is_empty());
    // A following turn consumes these together, in delivery order; the final explicit request wins.
    for (id, text) in [(72, "Translate the site into Italian"), (73, "Restore the original language"), (74, "Make the header blue")] {
        worker.notes.lock().unwrap().push(note(id, text));
        let left = notes::Left { text: text.into(), shown: text.into(), pictures: Vec::new(), added: None };
        notes::hand_over(&inbox, id, text, Some(&left)).unwrap();
    }
    let NotePrompt { prompt, .. } = worker.added(&mut ws, false, true).unwrap().unwrap();
    assert!(prompt.contains("Restore the original language"));
    assert!(notes::claim(&inbox).is_empty(), "a hook cannot deliver the same notes again");
    ws.reload().unwrap();
    website.record_requests(&mut ws, &[]).unwrap();
    assert_eq!(ws.manifest.website_language, Some(WebsiteLanguage::Original));
}

#[test]
fn website_language_write_failure_keeps_notes_and_refuses_the_check() {
    for claimed in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let mut ws = Workspace::create(dir.path(), "Site", "Rebuild this website in French", TargetRef::None).unwrap();
        let website = kind("website");
        website.record_requests(&mut ws, &[]).unwrap();
        let before = ws.manifest.website_language.clone();
        let mut worker = check_worker(&ws, false);
        worker.self_check = Some((website, 90.0));
        let inbox = notes::notes_dir(&ws.inbox_dir());
        worker.notes.lock().unwrap().push(note(71, "Translate the site into German"));
        notes::hand_over(&inbox, 71, "Translate the site into German", None).unwrap();
        if claimed {
            notes::claim(&inbox);
        }
        // A deterministic write refusal, including on privileged test runners.
        ws.manifest.version = craft_anything_engine::workspace::MANIFEST_VERSION + 1;
        let error = if claimed { worker.relay_read_notes(&mut ws).unwrap_err() } else { worker.added(&mut ws, false, true).unwrap_err() };
        assert_eq!(error.code, ErrorCode::NotSupported);
        assert_eq!(ws.manifest.website_language, before);
        assert_eq!(Workspace::open(&ws.root).unwrap().manifest.website_language, before);
        assert_eq!(worker.notes.lock().unwrap().len(), 1, "the failed save must not discard the note");
        assert!(if claimed { notes::was_read(&inbox, 71) } else { notes::claim(&inbox).len() == 1 });
    }
}

#[test]
fn claimed_language_notes_cannot_overtake_an_earlier_unread_observation() {
    let directory = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(directory.path(), "Site", "Rebuild this website in English", TargetRef::None).unwrap();
    let website = kind("website");
    website.record_requests(&mut ws, &[]).unwrap();
    let mut worker = check_worker(&ws, false);
    worker.self_check = Some((website, 90.0));
    let inbox = notes::notes_dir(&ws.inbox_dir());
    for (id, text) in [(71, "Translate the site into French"), (72, "Translate the site into German")] {
        worker.notes.lock().unwrap().push(note(id, text));
        notes::hand_over(&inbox, id, text, None).unwrap();
    }
    let mut observed = Vec::new();

    worker
        .relay_read_notes_using(&mut ws, |dir, id| {
            observed.push(id);
            let read = notes::was_read(dir, id);
            if id == 71 {
                assert!(!read);
                assert_eq!(notes::claim(dir).len(), 2, "the hook claims both between the two possible observations");
            }
            read
        })
        .unwrap();

    assert_eq!(observed, [71], "a later claim cannot pass the first unread observation");
    assert_eq!(worker.notes.lock().unwrap().iter().map(|note| note.id).collect::<Vec<_>>(), [71, 72]);
    assert_eq!(Workspace::open(&ws.root).unwrap().manifest.website_language, Some(WebsiteLanguage::Requested { tag: "en".into() }));
    worker.relay_read_notes(&mut ws).unwrap();
    assert!(worker.notes.lock().unwrap().is_empty());
    assert_eq!(Workspace::open(&ws.root).unwrap().manifest.website_language, Some(WebsiteLanguage::Requested { tag: "de".into() }));
}

#[test]
fn claimed_language_note_recording_mark_failure_keeps_unfinished_notes_retryable() {
    let directory = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(directory.path(), "Site", "Rebuild this website in English", TargetRef::None).unwrap();
    let website = kind("website");
    website.record_requests(&mut ws, &[]).unwrap();
    let mut worker = check_worker(&ws, false);
    worker.self_check = Some((website, 90.0));
    let inbox = notes::notes_dir(&ws.inbox_dir());
    for (id, text) in [(71, "Translate the site into French"), (72, "Translate the site into German")] {
        worker.notes.lock().unwrap().push(note(id, text));
        let left = notes::Left { text: text.into(), shown: text.into(), pictures: Vec::new(), added: None };
        notes::hand_over(&inbox, id, text, Some(&left)).unwrap();
    }
    assert_eq!(notes::claim(&inbox).len(), 2);
    std::fs::create_dir(inbox.join("72.recorded.json")).unwrap();

    assert!(worker.relay_read_notes(&mut ws).is_err());

    assert_eq!(worker.notes.lock().unwrap().iter().map(|note| note.id).collect::<Vec<_>>(), [72]);
    assert!(!notes::was_read(&inbox, 71), "the completed prefix is acknowledged and cannot enter the next handoff");
    assert!(inbox.join("72.read.json").is_file(), "a failed completion mark cannot forget the pending note");
    std::fs::remove_dir(inbox.join("72.recorded.json")).unwrap();
    let unfinished = inbox.join("72.read.json");
    let original = std::fs::read(&unfinished).unwrap();
    let mut future: Value = serde_json::from_slice(&original).unwrap();
    future["version"] = Value::from(99);
    let future = serde_json::to_vec(&future).unwrap();
    std::fs::write(&unfinished, &future).unwrap();
    assert!(worker.relay_read_notes(&mut ws).is_err(), "a live retry must still validate the unfinished record before cleanup");
    assert_eq!(std::fs::read(&unfinished).unwrap(), future);
    assert!(inbox.join("72.read.json").is_file());
    std::fs::write(&unfinished, original).unwrap();
    worker.relay_read_notes(&mut ws).unwrap();
    assert!(worker.notes.lock().unwrap().is_empty());
    assert_eq!(Workspace::open(&ws.root).unwrap().manifest.website_language, Some(WebsiteLanguage::Requested { tag: "de".into() }));
    assert!(notes::take_left(&inbox).unwrap().is_empty());
}

#[test]
fn website_language_changes_invalidate_a_kept_check() {
    use std::cell::Cell;
    let dir = tempfile::tempdir().unwrap();
    let mut ws = Workspace::create(dir.path(), "Site", "Rebuild this website in French", TargetRef::None).unwrap();
    std::fs::create_dir_all(ws.app_dir()).unwrap();
    std::fs::write(ws.app_dir().join("index.html"), "<html lang='fr'></html>").unwrap();
    let website = kind("website");
    website.record_requests(&mut ws, &[]).unwrap();
    let worker = check_worker(&ws, false);
    let calls = Cell::new(0);
    let check = |ws: &Workspace, round| {
        worker
            .checks_unless_unchanged(ws, website.as_ref(), round, |current| {
                calls.set(calls.get() + 1);
                Ok(vec![CheckResult::new(CheckResult::WEB_VISUAL, round, true, format!("{:?}", current.manifest.website_language))])
            })
            .unwrap()
    };
    assert!(check(&ws, 1)[0].note.contains("fr"));
    check(&ws, 2);
    assert_eq!(calls.get(), 1, "unchanged language keeps the successful round");
    let mut other = Workspace::open(&ws.root).unwrap();
    website.record_requests(&mut other, &["Translate the site into German"]).unwrap();
    ws.save().unwrap();
    assert_eq!(ws.manifest.website_language, Some(WebsiteLanguage::Requested { tag: "fr".into() }), "the run's handle is still stale");
    assert!(check(&ws, 3)[0].note.contains("de"), "the new request must be checked even when the build is unchanged");
    assert_eq!(calls.get(), 2);
    website.record_requests(&mut other, &["Restore the original language"]).unwrap();
    assert!(check(&ws, self_check::SELF_CHECK_ROUND)[0].note.contains("Original"));
    assert_eq!(calls.get(), 3, "resetting the language checks again too");
}
