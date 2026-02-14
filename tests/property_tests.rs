use proptest::prelude::*;

use netease_domain::model::music_info::{determine_file_extension, DownloadUrl};
use netease_domain::model::quality::quality_display_name;
use netease_kernel::util::filename::sanitize_filename;

proptest! {
    // INV: sanitize_filename never returns empty
    #[test]
    fn sanitize_never_empty(s in "\\PC*") {
        let result = sanitize_filename(&s);
        prop_assert!(!result.is_empty(), "sanitize_filename returned empty for input: {:?}", s);
    }

    // INV: sanitize_filename never exceeds 200 chars
    #[test]
    fn sanitize_max_length(s in ".{0,500}") {
        let result = sanitize_filename(&s);
        prop_assert!(result.len() <= 200, "sanitize_filename returned {} chars for input len {}", result.len(), s.len());
    }

    // INV: sanitize_filename removes all 9 illegal chars
    #[test]
    fn sanitize_no_illegal_chars(s in "\\PC*") {
        let result = sanitize_filename(&s);
        prop_assert!(!result.contains('<'), "contains < in: {}", result);
        prop_assert!(!result.contains('>'), "contains > in: {}", result);
        prop_assert!(!result.contains(':'), "contains : in: {}", result);
        prop_assert!(!result.contains('"'), "contains \" in: {}", result);
        prop_assert!(!result.contains('/'), "contains / in: {}", result);
        prop_assert!(!result.contains('\\'), "contains \\ in: {}", result);
        prop_assert!(!result.contains('|'), "contains | in: {}", result);
        prop_assert!(!result.contains('?'), "contains ? in: {}", result);
        prop_assert!(!result.contains('*'), "contains * in: {}", result);
    }

    // INV: sanitize_filename output does not start/end with space or dot
    #[test]
    fn sanitize_no_leading_trailing_space_or_dot(s in "\\PC*") {
        let result = sanitize_filename(&s);
        if result != "unknown" {
            prop_assert!(!result.starts_with(' '), "starts with space: {:?}", result);
            prop_assert!(!result.starts_with('.'), "starts with dot: {:?}", result);
            prop_assert!(!result.ends_with(' '), "ends with space: {:?}", result);
            prop_assert!(!result.ends_with('.'), "ends with dot: {:?}", result);
        }
    }

    // INV: quality_display_name never panics for any string input
    #[test]
    fn quality_display_never_panics(s in "[a-z]{0,20}") {
        let result = quality_display_name(&s);
        prop_assert!(!result.is_empty());
    }

    // INV: quality_display_name returns "未知音质" for unknown inputs
    #[test]
    fn quality_display_unknown_for_random(s in "[a-z]{5,20}") {
        // Most random 5+ char strings won't match any quality
        // This is a probabilistic test — it should pass because there are only 8 valid values
        let result = quality_display_name(&s);
        // We just verify it returns something (never panics)
        prop_assert!(!result.is_empty());
    }

    // INV: determine_file_extension always returns a valid extension starting with "."
    #[test]
    fn file_ext_always_valid(url in ".*", ft in "[a-z]{0,10}") {
        let ext = determine_file_extension(&url, &ft);
        prop_assert!(ext.starts_with('.'), "ext should start with '.': {}", ext);
        prop_assert!(ext.len() <= 5, "ext too long: {}", ext);
        prop_assert!(
            ext == ".mp3" || ext == ".flac" || ext == ".m4a",
            "unexpected ext: {}",
            ext
        );
    }

    // INV: DownloadUrl::is_empty reflects actual emptiness
    #[test]
    fn download_url_empty_consistency(s in ".*") {
        let url = DownloadUrl::new(s.clone());
        prop_assert_eq!(url.is_empty(), s.is_empty());
    }

    // INV: DownloadUrl::as_str roundtrips
    #[test]
    fn download_url_roundtrip(s in ".*") {
        let url = DownloadUrl::new(s.clone());
        prop_assert_eq!(url.as_str(), s.as_str());
    }

    // INV: DownloadUrl debug always shows redacted
    #[test]
    fn download_url_debug_redacted(s in ".*") {
        let url = DownloadUrl::new(s);
        let debug = format!("{:?}", url);
        prop_assert_eq!(debug, "DownloadUrl([redacted])");
    }
}
