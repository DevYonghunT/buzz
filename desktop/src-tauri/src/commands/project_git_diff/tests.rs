use super::{
    count_patch_changes, join_current_manifest, parse_current_name_status_z,
    parse_current_numstat_z, parse_current_unmerged_paths, parse_current_untracked_paths,
    parse_strict_numstat_counts, parse_untracked_patch_output, truncate_patch,
    CurrentChangesReadError, CurrentInventoryEntry, CurrentRepoChangeStatus, MAX_PATCH_BYTES,
    MAX_PATCH_LINES,
};

#[test]
fn patch_counts_header_shaped_content_inside_hunks() {
    let patch = concat!(
        "diff --git a/file b/file\n",
        "--- a/file\n",
        "+++ b/file\n",
        "@@ -1,2 +1,3 @@\n",
        "--- removed content\n",
        "+++ added content\n",
        "+normal addition\n",
    );

    assert_eq!(count_patch_changes(patch), (2, 1));
}

#[test]
fn patch_counts_each_type_change_section_without_counting_its_headers() {
    let patch = concat!(
        "diff --git a/file b/file\n",
        "deleted file mode 100644\n",
        "--- a/file\n",
        "+++ /dev/null\n",
        "@@ -1 +0,0 @@\n",
        "-regular\n",
        "diff --git a/file b/file\n",
        "new file mode 120000\n",
        "--- /dev/null\n",
        "+++ b/file\n",
        "@@ -0,0 +1 @@\n",
        "+target\n",
    );

    assert_eq!(count_patch_changes(patch), (1, 1));
}

#[test]
fn current_numstat_is_strict_and_preserves_binary_meaning() {
    assert_eq!(parse_strict_numstat_counts("-", "-"), Ok((0, 0, true)));
    assert_eq!(parse_strict_numstat_counts("12", "3"), Ok((12, 3, false)));
    assert!(parse_strict_numstat_counts("-", "3").is_err());
    assert!(parse_strict_numstat_counts("12", "-").is_err());
    assert!(parse_strict_numstat_counts("twelve", "3").is_err());

    let entries = parse_current_numstat_z(concat!("-\t-\tbinary.dat\0", "0\t0\tmode-only\0",))
        .expect("strict numstat should parse valid binary and zero-count entries");
    assert_eq!(entries.len(), 2);
    assert!(entries[0].binary);
    assert_eq!((entries[0].additions, entries[0].deletions), (0, 0));
    assert!(!entries[1].binary);
    assert!(parse_current_numstat_z("1\t0\tunterminated").is_err());
    assert!(parse_current_numstat_z("1\t0\0").is_err());
    assert!(parse_current_numstat_z(concat!("1\t0\tdup\0", "1\t0\tdup\0")).is_err());
    assert_eq!(
        parse_current_untracked_paths("replacement-\u{fffd}.txt\0")
            .expect("valid UTF-8 replacement characters are legal path content"),
        vec!["replacement-\u{fffd}.txt".to_string()]
    );
}

#[test]
fn current_name_status_is_closed_and_manifest_is_complete_sorted() {
    let statuses = parse_current_name_status_z(concat!(
        "M\0zeta.txt\0",
        "A\0alpha.txt\0",
        "D\0deleted.txt\0",
        "T\0typed.txt\0",
        "M\0conflict.txt\0",
    ))
    .expect("supported statuses should parse");
    assert_eq!(statuses[0].1, CurrentRepoChangeStatus::Modified);
    assert_eq!(statuses[1].1, CurrentRepoChangeStatus::Added);
    assert_eq!(statuses[2].1, CurrentRepoChangeStatus::Deleted);
    assert_eq!(statuses[3].1, CurrentRepoChangeStatus::TypeChanged);
    assert_eq!(statuses[4].1, CurrentRepoChangeStatus::Modified);
    assert_eq!(
        parse_current_name_status_z("U\0unmerged.txt\0")
            .expect("explicit unmerged status should remain supported")[0]
            .1,
        CurrentRepoChangeStatus::Unmerged
    );
    assert!(parse_current_name_status_z("R100\0old\0new\0").is_err());
    assert!(parse_current_name_status_z("M\0missing-terminator").is_err());

    let numstat = parse_current_numstat_z(concat!(
        "1\t0\tzeta.txt\0",
        "2\t0\talpha.txt\0",
        "0\t1\tdeleted.txt\0",
        "0\t0\ttyped.txt\0",
    ))
    .expect("tracked statistics should parse");
    let untracked = parse_current_untracked_paths("middle.txt\0beta.txt\0")
        .expect("untracked inventory should parse");
    let unmerged =
        parse_current_unmerged_paths("conflict.txt\0").expect("unmerged inventory should parse");
    let manifest = join_current_manifest(numstat, statuses, unmerged, untracked)
        .expect("matching full inventories should join");
    let paths = manifest
        .entries
        .iter()
        .map(|entry| match entry {
            CurrentInventoryEntry::Tracked(entry) => entry.path.as_str(),
            CurrentInventoryEntry::Untracked { path } => path.as_str(),
        })
        .collect::<Vec<_>>();
    assert_eq!(
        paths,
        vec![
            "alpha.txt",
            "beta.txt",
            "conflict.txt",
            "deleted.txt",
            "middle.txt",
            "typed.txt",
            "zeta.txt",
        ]
    );
    assert!(manifest.entries.iter().any(|entry| matches!(
        entry,
        CurrentInventoryEntry::Tracked(entry)
            if entry.path == "conflict.txt"
                && entry.status == CurrentRepoChangeStatus::Unmerged
    )));
}

#[test]
fn current_manifest_mismatches_fail_as_snapshot_drift() {
    let numstat =
        parse_current_numstat_z("1\t0\tonly-stat.txt\0").expect("test statistics should parse");
    assert!(matches!(
        join_current_manifest(numstat, Vec::new(), Vec::new(), Vec::new()),
        Err(CurrentChangesReadError::Drift)
    ));

    let statuses =
        parse_current_name_status_z("M\0only-status.txt\0").expect("test status should parse");
    assert!(matches!(
        join_current_manifest(Vec::new(), statuses, Vec::new(), Vec::new()),
        Err(CurrentChangesReadError::Drift)
    ));
}

#[test]
fn untracked_patch_classification_is_strict_and_ignores_hunk_marker_text() {
    let patch = concat!(
        "diff --git a/dev/fd/0 b/dev/fd/0\n",
        "new file mode 100644\n",
        "index 0000000..1111111\n",
        "--- /dev/null\n",
        "+++ b/dev/fd/0\n",
        "@@ -0,0 +1 @@\n",
        "+text\n",
    );
    assert_eq!(
        parse_untracked_patch_output(patch.to_string()),
        Ok((patch.to_string(), 1, 0, false))
    );
    let marker_in_hunk = patch.replace("+text", "+Binary files /dev/null and b/dev/fd/0 differ");
    assert!(parse_untracked_patch_output(marker_in_hunk).is_ok_and(
        |(_, additions, deletions, binary)| additions == 1 && deletions == 0 && !binary
    ));

    let binary_patch = concat!(
        "diff --git a/dev/fd/0 b/dev/fd/0\n",
        "new file mode 100644\n",
        "index 0000000..1111111\n",
        "Binary files /dev/null and b/dev/fd/0 differ\n",
    );
    assert_eq!(
        parse_untracked_patch_output(binary_patch.to_string()),
        Ok((binary_patch.to_string(), 0, 0, true))
    );
    let empty_patch = concat!(
        "diff --git a/dev/fd/0 b/dev/fd/0\n",
        "new file mode 100644\n",
    );
    assert_eq!(
        parse_untracked_patch_output(empty_patch.to_string()),
        Ok((empty_patch.to_string(), 0, 0, false))
    );
    let empty_patch_with_index = concat!(
        "diff --git a/dev/fd/0 b/dev/fd/0\n",
        "new file mode 100644\n",
        "index 000000000..e69de29bb\n",
    );
    assert_eq!(
        parse_untracked_patch_output(empty_patch_with_index.to_string()),
        Ok((empty_patch_with_index.to_string(), 0, 0, false))
    );
    assert!(parse_untracked_patch_output(
        binary_patch.replace("b/dev/fd/0 differ", "b/unexpected differ")
    )
    .is_err());
    assert!(parse_untracked_patch_output(format!("{binary_patch}@@ -0,0 +1 @@\n+text\n")).is_err());
}

#[test]
fn patch_truncation_respects_exact_line_and_utf8_byte_boundaries() {
    let exact = "line\n".repeat(MAX_PATCH_LINES);
    assert_eq!(truncate_patch(exact.clone()), (exact, false));

    let over = "line\n".repeat(MAX_PATCH_LINES + 1);
    let (retained, truncated) = truncate_patch(over);
    assert!(truncated);
    assert_eq!(retained.lines().count(), MAX_PATCH_LINES);

    let multibyte = "€".repeat((MAX_PATCH_BYTES / '€'.len_utf8()) + 1);
    let (retained, truncated) = truncate_patch(multibyte);
    assert!(truncated);
    assert!(retained.len() <= MAX_PATCH_BYTES);
    assert!(retained.chars().all(|character| character == '€'));
}
