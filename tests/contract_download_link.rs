use std::path::PathBuf;

use netease_domain::model::download::{DownloadResult, TaskInfo, TaskStage};
use netease_domain::model::music_info::{
    build_file_path, determine_file_extension, DownloadUrl, MusicInfo,
};
use netease_domain::model::quality::{VALID_QUALITIES, VALID_TYPES};

fn sample_music_info() -> MusicInfo {
    MusicInfo {
        id: 12345,
        name: "Test Song".into(),
        artists: "Artist A/Artist B".into(),
        album: "Test Album".into(),
        pic_url: "https://example.com/pic.jpg".into(),
        duration: 240,
        track_number: 1,
        download_url: DownloadUrl::new("https://m10.music.126.net/xxx.flac".into()),
        file_type: "flac".into(),
        file_size: 10_000_000,
        quality: "lossless".into(),
        lyric: "[00:00.00]Test lyric".into(),
        tlyric: "".into(),
    }
}

// --- Contract C-2: URL holding has no side effects ---

#[test]
fn c2_reading_download_url_is_pure() {
    let info = sample_music_info();

    // Reading as_str produces the stored value
    let url_str = info.download_url.as_str();
    assert_eq!(url_str, "https://m10.music.126.net/xxx.flac");

    // as_extension_hint also returns the URL
    let hint = info.download_url.as_extension_hint();
    assert_eq!(hint, url_str);
}

#[test]
fn c2_cloning_music_info_is_pure() {
    let info = sample_music_info();
    let cloned = info.clone();

    assert_eq!(cloned.download_url.as_str(), info.download_url.as_str());
    assert_eq!(cloned.name, info.name);
    assert_eq!(cloned.id, info.id);
}

#[test]
fn c2_build_file_path_does_not_consume_url() {
    let info = sample_music_info();
    let dir = PathBuf::from("/tmp/downloads");

    // Building file path should only use extension hint, not consume URL
    let path = build_file_path(&dir, &info, "lossless");
    assert!(path.to_string_lossy().contains("lossless"));
    assert!(path.to_string_lossy().ends_with(".flac"));

    // URL is still accessible
    assert_eq!(
        info.download_url.as_str(),
        "https://m10.music.126.net/xxx.flac"
    );
}

#[test]
fn c2_debug_format_redacts_url() {
    let url = DownloadUrl::new("https://secret.cdn.example.com/file.mp3".into());
    let debug_output = format!("{:?}", url);
    assert_eq!(debug_output, "DownloadUrl([redacted])");
    assert!(!debug_output.contains("secret"));
}

// --- Contract C-5: Dedup guarantee (TaskStage-based) ---

#[test]
fn c5_non_terminal_stages_should_prevent_new_task() {
    let non_terminal = [
        TaskStage::Starting,
        TaskStage::FetchingUrl,
        TaskStage::Downloading,
        TaskStage::Packaging,
    ];

    for stage in non_terminal {
        assert!(
            !stage.is_terminal(),
            "{:?} should not be terminal (dedup should block new task)",
            stage
        );
    }
}

#[test]
fn c5_terminal_stages_allow_new_task() {
    // Error and Retrieved allow creating a new task for the same key
    assert!(TaskStage::Error.is_terminal());
    assert!(TaskStage::Retrieved.is_terminal());
    assert!(TaskStage::Done.is_terminal());
}

// --- Contract C-6: Task result single retrieval ---

#[test]
fn c6_done_to_retrieved_is_valid_transition() {
    let mut task = TaskInfo::new();
    assert_eq!(task.stage, TaskStage::Starting);

    // Simulate progression to Done
    task.stage = TaskStage::Done;
    task.percent = 100;
    task.zip_path = Some("/tmp/test.zip".into());
    task.zip_filename = Some("test.zip".into());

    // First retrieval: done -> retrieved
    assert_eq!(task.stage, TaskStage::Done);
    let first_access = task.stage == TaskStage::Done;
    assert!(first_access);
    task.stage = TaskStage::Retrieved;

    // Second retrieval: stays retrieved
    assert_eq!(task.stage, TaskStage::Retrieved);
    let second_first_access = task.stage == TaskStage::Done;
    assert!(!second_first_access);
}

// --- DownloadResult invariants ---

#[test]
fn download_result_ok_invariants() {
    let info = sample_music_info();
    let result = DownloadResult::ok(PathBuf::from("/tmp/test.flac"), 1024, info);

    assert!(result.success);
    assert!(result.file_path.is_some());
    assert!(result.music_info.is_some());
    assert!(result.error_message.is_empty());
    assert_eq!(result.file_size, 1024);
    assert!(result.cover_data.is_none());
}

#[test]
fn download_result_ok_with_cover_invariants() {
    let info = sample_music_info();
    let cover = Some(vec![0xFF, 0xD8, 0xFF]);
    let result = DownloadResult::ok_with_cover(PathBuf::from("/tmp/test.flac"), 2048, info, cover);

    assert!(result.success);
    assert!(result.file_path.is_some());
    assert!(result.music_info.is_some());
    assert!(result.cover_data.is_some());
    assert_eq!(result.cover_data.unwrap().len(), 3);
}

#[test]
fn download_result_fail_invariants() {
    let result = DownloadResult::fail("something went wrong");

    assert!(!result.success);
    assert!(result.file_path.is_none());
    assert!(result.music_info.is_none());
    assert!(result.cover_data.is_none());
    assert_eq!(result.file_size, 0);
    assert_eq!(result.error_message, "something went wrong");
}

// --- Quality validation ---

#[test]
fn valid_qualities_contains_expected_values() {
    let expected = [
        "standard", "exhigh", "lossless", "hires", "sky", "jyeffect", "jymaster", "dolby",
    ];
    assert_eq!(VALID_QUALITIES.len(), expected.len());
    for q in &expected {
        assert!(VALID_QUALITIES.contains(q), "Missing quality: {}", q);
    }
}

#[test]
fn valid_types_contains_expected_values() {
    let expected = ["url", "name", "lyric", "json"];
    assert_eq!(VALID_TYPES.len(), expected.len());
    for t in &expected {
        assert!(VALID_TYPES.contains(t), "Missing type: {}", t);
    }
}

#[test]
fn dolby_in_valid_qualities_and_has_display_name() {
    assert!(VALID_QUALITIES.contains(&"dolby"));
    assert_eq!(
        netease_domain::model::quality::quality_display_name("dolby"),
        "杜比全景声"
    );
}

// --- File extension determination ---

#[test]
fn determine_extension_flac_by_url() {
    assert_eq!(
        determine_file_extension("https://cdn.com/file.FLAC?token=abc", "mp3"),
        ".flac"
    );
}

#[test]
fn determine_extension_flac_by_type() {
    assert_eq!(
        determine_file_extension("https://cdn.com/file", "flac"),
        ".flac"
    );
}

#[test]
fn determine_extension_m4a_by_url() {
    assert_eq!(
        determine_file_extension("https://cdn.com/file.m4a", ""),
        ".m4a"
    );
}

#[test]
fn determine_extension_m4a_by_type() {
    assert_eq!(
        determine_file_extension("https://cdn.com/file", "m4a"),
        ".m4a"
    );
}

#[test]
fn determine_extension_defaults_to_mp3() {
    assert_eq!(determine_file_extension("https://cdn.com/file", ""), ".mp3");
    assert_eq!(determine_file_extension("", ""), ".mp3");
    assert_eq!(determine_file_extension("", "ogg"), ".mp3");
}

// --- build_file_path ---

#[test]
fn build_file_path_includes_quality_dir() {
    let info = sample_music_info();
    let path = build_file_path(&PathBuf::from("/downloads"), &info, "hires");
    let path_str = path.to_string_lossy();

    assert!(
        path_str.contains("hires"),
        "Path should contain quality dir"
    );
    assert!(
        path_str.ends_with(".flac"),
        "Path should have .flac extension"
    );
    assert!(
        path_str.contains("Test Song - Artist A_Artist B"),
        "Path should contain sanitized filename"
    );
}

// --- TaskInfo ---

#[test]
fn task_info_new_defaults() {
    let task = TaskInfo::new();
    assert_eq!(task.stage, TaskStage::Starting);
    assert_eq!(task.percent, 0);
    assert!(task.zip_path.is_none());
    assert!(task.zip_filename.is_none());
    assert!(task.error.is_none());
    assert!(task.current.is_none());
    assert!(task.total.is_none());
    assert!(task.completed.is_none());
    assert!(task.failed.is_none());
    assert!(task.created_at > 0);
}

// --- DownloadUrl ---

#[test]
fn download_url_empty_check() {
    let empty = DownloadUrl::new(String::new());
    assert!(empty.is_empty());

    let non_empty = DownloadUrl::new("https://example.com".into());
    assert!(!non_empty.is_empty());
}
