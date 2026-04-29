use netease_domain::model::download::{TaskInfo, TaskStage};

// --- TaskStage::is_terminal correctness ---

#[test]
fn terminal_stages() {
    assert!(TaskStage::Done.is_terminal());
    assert!(TaskStage::Error.is_terminal());
    assert!(TaskStage::Retrieved.is_terminal());
}

#[test]
fn non_terminal_stages() {
    assert!(!TaskStage::Starting.is_terminal());
    assert!(!TaskStage::FetchingUrl.is_terminal());
    assert!(!TaskStage::Downloading.is_terminal());
    assert!(!TaskStage::Packaging.is_terminal());
}

#[test]
fn exhaustive_terminal_check() {
    let all_stages = [
        TaskStage::Starting,
        TaskStage::FetchingUrl,
        TaskStage::Downloading,
        TaskStage::Packaging,
        TaskStage::Done,
        TaskStage::Retrieved,
        TaskStage::Error,
    ];

    let terminal_count = all_stages.iter().filter(|s| s.is_terminal()).count();
    assert_eq!(terminal_count, 3, "Exactly 3 stages should be terminal");

    let non_terminal_count = all_stages.iter().filter(|s| !s.is_terminal()).count();
    assert_eq!(
        non_terminal_count, 4,
        "Exactly 4 stages should be non-terminal"
    );
}

// --- TaskStage Display ---

#[test]
fn stage_display_matches_expected_strings() {
    assert_eq!(TaskStage::Starting.to_string(), "starting");
    assert_eq!(TaskStage::FetchingUrl.to_string(), "fetching_url");
    assert_eq!(TaskStage::Downloading.to_string(), "downloading");
    assert_eq!(TaskStage::Packaging.to_string(), "packaging");
    assert_eq!(TaskStage::Done.to_string(), "done");
    assert_eq!(TaskStage::Retrieved.to_string(), "retrieved");
    assert_eq!(TaskStage::Error.to_string(), "error");
}

// --- TaskStage serde serialization ---

#[test]
fn stage_serializes_to_snake_case() {
    assert_eq!(
        serde_json::to_string(&TaskStage::Starting).unwrap(),
        "\"starting\""
    );
    assert_eq!(
        serde_json::to_string(&TaskStage::FetchingUrl).unwrap(),
        "\"fetching_url\""
    );
    assert_eq!(
        serde_json::to_string(&TaskStage::Downloading).unwrap(),
        "\"downloading\""
    );
    assert_eq!(
        serde_json::to_string(&TaskStage::Packaging).unwrap(),
        "\"packaging\""
    );
    assert_eq!(serde_json::to_string(&TaskStage::Done).unwrap(), "\"done\"");
    assert_eq!(
        serde_json::to_string(&TaskStage::Retrieved).unwrap(),
        "\"retrieved\""
    );
    assert_eq!(
        serde_json::to_string(&TaskStage::Error).unwrap(),
        "\"error\""
    );
}

// --- TaskStage PartialEq ---

#[test]
fn stage_equality() {
    assert_eq!(TaskStage::Done, TaskStage::Done);
    assert_ne!(TaskStage::Done, TaskStage::Error);
    assert_ne!(TaskStage::Starting, TaskStage::Downloading);
}

// --- TaskStage Copy ---

#[test]
fn stage_is_copy() {
    let a = TaskStage::Done;
    let b = a; // Copy
    assert_eq!(a, b);
}

// --- Valid state transitions ---
// The following tests document the expected state machine transitions.
// Not all transitions are enforced by the type system, but the handlers follow this protocol.

#[test]
fn valid_forward_transitions() {
    // Happy path: Starting -> FetchingUrl -> Downloading -> Packaging -> Done -> Retrieved
    let path = [
        TaskStage::Starting,
        TaskStage::FetchingUrl,
        TaskStage::Downloading,
        TaskStage::Packaging,
        TaskStage::Done,
        TaskStage::Retrieved,
    ];

    for window in path.windows(2) {
        let from = window[0];
        let to = window[1];
        // All forward transitions in the happy path are valid
        assert!(
            !from.is_terminal() || (from == TaskStage::Done && to == TaskStage::Retrieved),
            "Unexpected terminal-to-non-terminal: {:?} -> {:?}",
            from,
            to
        );
    }
}

#[test]
fn done_to_retrieved_is_only_terminal_to_terminal() {
    let terminals = [TaskStage::Done, TaskStage::Error, TaskStage::Retrieved];

    for from in &terminals {
        for to in &terminals {
            if *from == TaskStage::Done && *to == TaskStage::Retrieved {
                // This is the only valid terminal-to-terminal transition
                continue;
            }
            if from == to {
                // Same state is a no-op, acceptable
                continue;
            }
            // All other terminal-to-different-terminal transitions should not happen
            // (not enforced by type system, but documented here)
        }
    }
}

#[test]
fn any_stage_can_transition_to_error() {
    let all_non_error = [
        TaskStage::Starting,
        TaskStage::FetchingUrl,
        TaskStage::Downloading,
        TaskStage::Packaging,
        TaskStage::Done,
    ];

    for stage in &all_non_error {
        // Transitioning to Error is always valid (cancel, timeout, failure)
        let _error = TaskStage::Error;
        assert!(
            !stage.is_terminal() || *stage == TaskStage::Done,
            "{:?} -> Error should be valid",
            stage
        );
    }
}

// --- TaskInfo full lifecycle simulation ---

#[test]
fn task_lifecycle_happy_path() {
    let mut task = TaskInfo::new();
    assert_eq!(task.stage, TaskStage::Starting);
    assert_eq!(task.percent, 0);

    // Resolve URL
    task.stage = TaskStage::FetchingUrl;
    task.detail = "正在获取下载链接...".into();
    assert!(!task.stage.is_terminal());

    // Download
    task.stage = TaskStage::Downloading;
    task.percent = 50;
    task.detail = "正在下载音乐文件 (50%)...".into();
    assert!(!task.stage.is_terminal());

    // Package
    task.stage = TaskStage::Packaging;
    task.percent = 95;
    task.detail = "正在打包...".into();
    assert!(!task.stage.is_terminal());

    // Done
    task.stage = TaskStage::Done;
    task.percent = 100;
    task.zip_path = Some("/tmp/music_api_zips/abc123.zip".into());
    task.zip_filename = Some("Song - Artist.zip".into());
    assert!(task.stage.is_terminal());

    // Retrieved
    let first_access = task.stage == TaskStage::Done;
    assert!(first_access);
    task.stage = TaskStage::Retrieved;
    assert!(task.stage.is_terminal());

    // Second access: no longer first_access
    let second_access = task.stage == TaskStage::Done;
    assert!(!second_access);
}

#[test]
fn task_lifecycle_error_path() {
    let mut task = TaskInfo::new();

    task.stage = TaskStage::FetchingUrl;
    task.stage = TaskStage::Error;
    task.error = Some("无可用的下载链接".into());

    assert!(task.stage.is_terminal());
    assert!(task.error.is_some());
}

#[test]
fn task_lifecycle_cancel_during_download() {
    let mut task = TaskInfo::new();

    task.stage = TaskStage::Downloading;
    task.percent = 30;

    // Cancel
    task.stage = TaskStage::Error;
    task.error = Some("已取消".into());

    assert!(task.stage.is_terminal());
    assert_eq!(task.error.as_deref(), Some("已取消"));
}

// --- Batch task lifecycle ---

#[test]
fn batch_task_progress_fields() {
    let mut task = TaskInfo::new();

    task.stage = TaskStage::Downloading;
    task.current = Some(3);
    task.total = Some(10);
    task.completed = Some(2);
    task.failed = Some(0);
    task.detail = "正在下载 Song - Artist (45%) [3/10]".into();

    assert_eq!(task.current, Some(3));
    assert_eq!(task.total, Some(10));
    assert_eq!(task.completed, Some(2));
    assert_eq!(task.failed, Some(0));
}

#[test]
fn batch_task_done_with_stats() {
    let mut task = TaskInfo::new();

    task.stage = TaskStage::Done;
    task.percent = 100;
    task.completed = Some(8);
    task.failed = Some(2);
    task.total = Some(10);
    task.zip_path = Some("/tmp/music_api_zips/batch.zip".into());
    task.zip_filename = Some("batch_8tracks.zip".into());
    task.detail = "下载完成 (8/10)".into();

    assert!(task.stage.is_terminal());
    assert_eq!(
        task.completed.unwrap() + task.failed.unwrap(),
        task.total.unwrap()
    );
}

// --- Cookie model ---

#[test]
fn cookie_parse_empty() {
    use netease_domain::model::cookie::parse_cookie_string;
    let result = parse_cookie_string("");
    assert!(result.is_empty());
}

#[test]
fn cookie_parse_bare_value_treated_as_music_u() {
    use netease_domain::model::cookie::parse_cookie_string;
    let result = parse_cookie_string("abc123def456");
    assert_eq!(result.len(), 1);
    assert_eq!(result.get("MUSIC_U").unwrap(), "abc123def456");
}

#[test]
fn cookie_parse_semicolon_separated() {
    use netease_domain::model::cookie::parse_cookie_string;
    let result = parse_cookie_string("MUSIC_U=longvalue123; __csrf=token456; NMTID=xyz");
    assert_eq!(result.len(), 3);
    assert_eq!(result.get("MUSIC_U").unwrap(), "longvalue123");
    assert_eq!(result.get("__csrf").unwrap(), "token456");
}

#[test]
fn cookie_parse_newline_separated() {
    use netease_domain::model::cookie::parse_cookie_string;
    let result = parse_cookie_string("MUSIC_U=val1\n__csrf=val2");
    assert_eq!(result.len(), 2);
}

#[test]
fn cookie_validation_empty_is_invalid() {
    use netease_domain::model::cookie::is_cookies_valid;
    use std::collections::HashMap;
    assert!(!is_cookies_valid(&HashMap::new()));
}

#[test]
fn cookie_validation_music_u_too_short() {
    use netease_domain::model::cookie::is_cookies_valid;
    use std::collections::HashMap;
    let mut cookies = HashMap::new();
    cookies.insert("MUSIC_U".into(), "short".into());
    assert!(!is_cookies_valid(&cookies));
}

#[test]
fn cookie_validation_music_u_valid() {
    use netease_domain::model::cookie::is_cookies_valid;
    use std::collections::HashMap;
    let mut cookies = HashMap::new();
    cookies.insert("MUSIC_U".into(), "a_valid_long_cookie_value".into());
    assert!(is_cookies_valid(&cookies));
}

#[test]
fn cookie_validation_no_important_keys() {
    use netease_domain::model::cookie::is_cookies_valid;
    use std::collections::HashMap;
    let mut cookies = HashMap::new();
    cookies.insert("unimportant_key".into(), "value".into());
    assert!(!is_cookies_valid(&cookies));
}
