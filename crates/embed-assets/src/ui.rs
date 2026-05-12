use std::io::{self, BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};

pub fn prompt_dir() -> Result<PathBuf> {
    println!("embed-assets — 同名 jpg/lrc 嵌入音频 tag");
    loop {
        print!("请输入要扫描的文件夹路径（回车=当前目录，q 退出）: ");
        io::stdout().flush().ok();
        let mut line = String::new();
        io::stdin()
            .lock()
            .read_line(&mut line)
            .context("read folder path")?;
        let raw = line.trim();
        if raw.eq_ignore_ascii_case("q") {
            std::process::exit(0);
        }
        let cleaned = strip_quotes(raw);
        let path = if cleaned.is_empty() {
            PathBuf::from(".")
        } else {
            PathBuf::from(cleaned)
        };
        if path.is_dir() {
            return Ok(path);
        }
        println!("  路径不存在或不是文件夹：{}", path.display());
    }
}

fn strip_quotes(s: &str) -> &str {
    let bytes = s.as_bytes();
    if bytes.len() >= 2
        && ((bytes[0] == b'"' && bytes[bytes.len() - 1] == b'"')
            || (bytes[0] == b'\'' && bytes[bytes.len() - 1] == b'\''))
    {
        &s[1..s.len() - 1]
    } else {
        s
    }
}

pub fn pause() {
    print!("\n按回车键退出...");
    io::stdout().flush().ok();
    let mut buf = String::new();
    let _ = io::stdin().lock().read_line(&mut buf);
}
