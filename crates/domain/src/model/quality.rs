pub const VALID_QUALITIES: &[&str] = &[
    "standard", "exhigh", "lossless", "hires", "sky", "jyeffect", "jymaster", "dolby",
];

pub const VALID_TYPES: &[&str] = &["url", "name", "lyric", "json"];

pub fn quality_display_name(quality: &str) -> &'static str {
    match quality {
        "standard" => "标准音质",
        "exhigh" => "极高音质",
        "lossless" => "无损音质",
        "hires" => "Hi-Res音质",
        "sky" => "沉浸环绕声",
        "jyeffect" => "高清环绕声",
        "jymaster" => "超清母带",
        "dolby" => "杜比全景声",
        _ => "未知音质",
    }
}
