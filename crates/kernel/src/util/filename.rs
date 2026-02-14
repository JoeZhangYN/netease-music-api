pub fn sanitize_filename(filename: &str) -> String {
    let sanitized: String = filename
        .chars()
        .map(|c| match c {
            '<' | '>' | ':' | '"' | '/' | '\\' | '|' | '?' | '*' => '_',
            _ => c,
        })
        .collect();

    let trimmed = sanitized.trim_matches(|c: char| c == ' ' || c == '.');
    let result = if trimmed.len() > 200 {
        let mut end = 200;
        while !trimmed.is_char_boundary(end) {
            end -= 1;
        }
        &trimmed[..end]
    } else {
        trimmed
    };

    if result.is_empty() {
        "unknown".to_string()
    } else {
        result.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sanitize_basic() {
        assert_eq!(sanitize_filename("hello world"), "hello world");
    }

    #[test]
    fn test_sanitize_illegal_chars() {
        assert_eq!(sanitize_filename("a<b>c:d"), "a_b_c_d");
    }

    #[test]
    fn test_sanitize_empty() {
        assert_eq!(sanitize_filename("..."), "unknown");
    }

    #[test]
    fn test_sanitize_long() {
        let long = "a".repeat(300);
        assert_eq!(sanitize_filename(&long).len(), 200);
    }
}
