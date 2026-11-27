//! The build script joins each language's catalog folder (`build.rs`), so a change that adds
//! strings can put them in a file of its own instead of appending to one shared file, where any
//! two such changes conflicted (#1465).

#[allow(dead_code)]
#[path = "../build.rs"]
mod build;

#[test]
fn a_languages_catalog_files_are_joined_in_name_order() {
    let src = tempfile::tempdir().unwrap();
    let out = tempfile::tempdir().unwrap();
    std::fs::create_dir_all(src.path().join("de")).unwrap();
    // The last line of a file without a line break must not run into the next file's first.
    std::fs::write(src.path().join("de/r5.4-budget.tsv"), "Budget\tBudget").unwrap();
    std::fs::write(src.path().join("de/core.tsv"), "# de\nSettings\tEinstellungen\n").unwrap();
    std::fs::write(src.path().join("de/notes.txt"), "not a catalog\tnope\n").unwrap();
    std::fs::write(src.path().join("awaiting.txt"), "Budget\n").unwrap();
    // A catalog left from a build when a language's folder still existed.
    std::fs::write(
        out.path().join("fr.tsv"),
        "Settings	Paramètres
",
    )
    .unwrap();
    build::join_catalogs(src.path(), out.path());
    let joined = std::fs::read_to_string(out.path().join("de.tsv")).unwrap();
    assert_eq!(joined, "# de\nSettings\tEinstellungen\nBudget\tBudget\n");
    assert!(!out.path().join("awaiting.tsv").exists(), "only folders are catalogs");
    assert!(!out.path().join("fr.tsv").exists(), "a removed language's old catalog is cleared");
}

/// Review of #1860: a key in two files of one language is refused, naming both files, rather
/// than the joined catalog keeping whichever sorts last.
#[test]
fn a_key_in_two_files_of_a_language_is_refused() {
    let src = tempfile::tempdir().unwrap();
    let (core, extra) = (src.path().join("core.tsv"), src.path().join("r5.4-budget.tsv"));
    std::fs::write(&core, "# de\nSettings\tEinstellungen\n").unwrap();
    std::fs::write(&extra, "Budget\tBudget\n").unwrap();
    assert!(build::join_files(&[core.clone(), extra.clone()]).is_ok(), "different keys join");
    std::fs::write(&extra, "Budget\tBudget\nSettings\tOptionen\n").unwrap();
    let problems = build::join_files(&[core.clone(), extra.clone()]).unwrap_err();
    assert_eq!(problems.len(), 1, "{problems:?}");
    assert!(problems[0].contains("\"Settings\"") && problems[0].contains("core.tsv") && problems[0].contains("r5.4-budget.tsv"), "{problems:?}");
}

/// Every language the app offers comes from its folder, so no catalog is a single shared file.
#[test]
fn every_language_is_a_folder() {
    let src = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for lang in craft_anything_ui_egui::i18n::LANGUAGES.iter().filter(|l| l.code != "en") {
        assert!(src.join("i18n").join(lang.code).is_dir(), "{} has no catalog folder", lang.code);
        assert!(
            !src.join(format!("{}.tsv", lang.code)).exists() && !src.join("i18n").join(format!("{}.tsv", lang.code)).exists(),
            "{} still has a single file",
            lang.code
        );
    }
}
