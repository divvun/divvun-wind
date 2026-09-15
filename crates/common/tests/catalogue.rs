use std::fs;
use wind_common::catalogue::{check_file, parse_file, Loader, MAX_FILE_BYTES};

const KILDIN: &str = r#"{"sjd-Cyrl":{"abbreviation":"КИЛ","display_name":{"en":"Kildin Sami","ru":"Кильдинский саамский"}}}"#;

#[test]
fn arbitrary_filenames_bom_and_localized_labels() {
    let dir = tempfile::tempdir().unwrap();
    let filename = dir.path().join("installer chosen Кильдинский 42.JSON");
    fs::write(
        &filename,
        [b"\xef\xbb\xbf".as_slice(), KILDIN.as_bytes()].concat(),
    )
    .unwrap();
    fs::write(dir.path().join("not-a-candidate.tmp"), "broken").unwrap();
    fs::create_dir(dir.path().join("nested.json")).unwrap();
    fs::write(dir.path().join("nested.json/ignored.json"), "broken").unwrap();
    let snapshot = Loader::default().scan(dir.path());
    assert!(snapshot.valid());
    assert_eq!(snapshot.entries.len(), 1);
    assert_eq!(snapshot.sources["sjd-cyrl"], vec![filename]);
    let entry = &snapshot.entries["sjd-cyrl"];
    assert_eq!(
        entry.abbreviation.as_ref().unwrap().resolve(["ru-RU"]),
        Some("КИЛ")
    );
    assert_eq!(
        entry.display_name.as_ref().unwrap().resolve(["ru-BY"]),
        Some("Кильдинский саамский")
    );
    assert_eq!(
        entry.display_name.as_ref().unwrap().resolve(["sv-SE"]),
        Some("Kildin Sami")
    );
}

#[test]
fn rejects_duplicate_members_before_maps_can_discard_them() {
    for input in [
        r#"{"sjd":{"abbreviation":"A"},"SJD":{"abbreviation":"A"}}"#,
        r#"{"sjd":{"abbreviation":"A","abbreviation":"B"}}"#,
        r#"{"sjd":{"display_name":{"en":"A","EN":"A"}}}"#,
    ] {
        assert!(parse_file(input.as_bytes())
            .unwrap_err()
            .contains("duplicate member"));
    }
}

#[test]
fn schema_is_strict_but_does_not_require_registered_languages() {
    for input in [
        r#"{"x-unlisted":{"abbreviation":"NEW"}}"#,
        r#"{"qzz-Latn":{"display_name":{"en":"Unknown"}}}"#,
        r#"{}"#,
    ] {
        assert!(parse_file(input.as_bytes()).is_ok(), "{input}");
    }
    for input in [
        r#"[]"#,
        r#"{"sjd":{}}"#,
        r#"{"sjd":{"abbreviation":null}}"#,
        r#"{"sjd":{"abbreviation":""}}"#,
        r#"{"sjd":{"abbreviation":"\u0000"}}"#,
        r#"{"sjd":{"display_name":{}}}"#,
        r#"{"sjd":{"abbreviation":"A","extra":true}}"#,
        r#"{"sjd_Cyrl":{"abbreviation":"A"}}"#,
        r#"{"sjd":{"display_name":{"en_US":"A"}}}"#,
    ] {
        assert!(parse_file(input.as_bytes()).is_err(), "{input}");
    }
    assert!(parse_file(&[0xff]).is_err());
}

#[test]
fn identical_entries_coalesce_and_conflicts_disable_only_their_tag() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("z.json"), KILDIN).unwrap();
    fs::write(
        dir.path().join("a.json"),
        KILDIN
            .replace("sjd-Cyrl", "SJD-CYRL")
            .replace("\"en\"", "\"EN\""),
    )
    .unwrap();
    let mut loader = Loader::default();
    let same = loader.scan(dir.path());
    assert!(same.valid());
    assert_eq!(same.sources["sjd-cyrl"].len(), 2);
    assert_eq!(same.diagnostics[0].code, "identical_tag");
    fs::write(
        dir.path().join("b.json"),
        r#"{"sjd-cyrl":{"abbreviation":"DIFFERENT"},"sjd":{"abbreviation":"BASE"}}"#,
    )
    .unwrap();
    let conflict = loader.scan(dir.path());
    assert!(!conflict.valid());
    assert!(!conflict.entries.contains_key("sjd-cyrl"));
    assert!(conflict.entries.contains_key("sjd"));
    assert_eq!(conflict.diagnostics[0].paths.len(), 3);
    fs::remove_file(dir.path().join("b.json")).unwrap();
    assert!(loader.scan(dir.path()).valid());
}

#[test]
fn literal_and_translation_map_are_distinct_definitions() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(
        dir.path().join("a.json"),
        r#"{"sjd":{"display_name":"Name"}}"#,
    )
    .unwrap();
    fs::write(
        dir.path().join("b.json"),
        r#"{"sjd":{"display_name":{"en":"Name"}}}"#,
    )
    .unwrap();
    let snapshot = Loader::default().scan(dir.path());
    assert!(!snapshot.valid());
    assert!(snapshot.entries.is_empty());
}

#[test]
fn damaged_file_is_stale_until_fixed_and_confirmed_deletion_removes_it() {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("keyboard.json");
    fs::write(&file, KILDIN).unwrap();
    let mut loader = Loader::default();
    let original = loader.scan(dir.path());
    fs::write(&file, "{").unwrap();
    fs::write(dir.path().join("new-invalid.json"), "{").unwrap();
    let stale = loader.scan(dir.path());
    assert!(!stale.valid());
    assert_eq!(stale.entries, original.entries);
    assert_eq!(stale.stale_files, vec![file.clone()]);
    assert_eq!(stale.diagnostics.len(), 2);
    fs::remove_file(&file).unwrap();
    assert!(loader.scan(dir.path()).entries.is_empty());
    fs::write(&file, KILDIN).unwrap();
    fs::remove_file(dir.path().join("new-invalid.json")).unwrap();
    assert!(loader.scan(dir.path()).valid());
    assert!(loader.scan(dir.path()).stale_files.is_empty());
}

#[test]
fn missing_directory_is_empty_but_enumeration_failure_retains_previous_view() {
    let dir = tempfile::tempdir().unwrap();
    let catalogue = dir.path().join("catalogue");
    let mut loader = Loader::default();
    assert!(loader.scan(&catalogue).valid());
    fs::create_dir(&catalogue).unwrap();
    fs::write(catalogue.join("a.json"), KILDIN).unwrap();
    let original = loader.scan(&catalogue);
    fs::rename(&catalogue, dir.path().join("saved")).unwrap();
    fs::write(&catalogue, "this is a file").unwrap();
    let unavailable = loader.scan(&catalogue);
    assert!(!unavailable.scan_complete);
    assert_eq!(unavailable.entries, original.entries);
    fs::remove_file(&catalogue).unwrap();
    let deleted = loader.scan(&catalogue);
    assert!(deleted.valid());
    assert!(deleted.entries.is_empty());
}

#[test]
fn oversized_files_are_isolated_and_candidate_and_aggregate_limits_fail_the_scan() {
    let dir = tempfile::tempdir().unwrap();
    fs::write(dir.path().join("good.json"), KILDIN).unwrap();
    fs::write(dir.path().join("huge.json"), vec![b' '; MAX_FILE_BYTES + 1]).unwrap();
    assert!(!check_file(&dir.path().join("huge.json")).valid());
    let mut loader = Loader::default();
    let initial = loader.scan(dir.path());
    assert!(initial.scan_complete);
    assert_eq!(initial.entries.len(), 1);
    // Whitespace still counts towards the read budget, even for invalid JSON.
    for i in 0..33 {
        fs::write(
            dir.path().join(format!("data-{i}.json")),
            vec![b' '; MAX_FILE_BYTES],
        )
        .unwrap();
    }
    let aggregate = loader.scan(dir.path());
    assert!(!aggregate.scan_complete);
    assert_eq!(aggregate.entries, initial.entries);
    let many = tempfile::tempdir().unwrap();
    for i in 0..1025 {
        fs::write(many.path().join(format!("{i}.json")), "{}").unwrap();
    }
    assert!(!Loader::default().scan(many.path()).scan_complete);
}
