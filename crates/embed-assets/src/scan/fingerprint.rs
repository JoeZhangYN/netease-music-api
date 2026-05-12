use std::collections::hash_map::DefaultHasher;
use std::fs;
use std::hash::Hasher;
use std::io::{BufReader, Read};
use std::path::{Path, PathBuf};

/// 检查多个音频文件是否字节级完全相同（同 size + 同内容 hash）。
/// 用于「多音频同 stem」场景：用户把同一首歌散落到多个目录的副本应静默全合并，
/// 真正不同的同名文件才提示用户选。
///
/// 性能：先比 size（廉价 metadata 调用）；size 全相等才读文件算 hash。
/// hash 用 std SipHash（DefaultHasher）—— 非加密但同进程内自洽，识别相同副本足够。
pub fn check_all_identical(audios: &[PathBuf]) -> bool {
    if audios.len() < 2 {
        return true;
    }

    let mut sizes = Vec::with_capacity(audios.len());
    for p in audios {
        match fs::metadata(p) {
            Ok(m) => sizes.push(m.len()),
            Err(_) => return false,
        }
    }
    if sizes.windows(2).any(|w| w[0] != w[1]) {
        return false;
    }

    let mut prev: Option<u64> = None;
    for p in audios {
        let Some(h) = file_hash(p) else {
            return false;
        };
        match prev {
            Some(p0) if p0 != h => return false,
            None => prev = Some(h),
            _ => {}
        }
    }
    true
}

/// 两文件字节级相同（size + 内容 hash）。任一文件读不到都按"不同"处理。
pub fn files_identical(a: &Path, b: &Path) -> bool {
    let (Ok(ma), Ok(mb)) = (fs::metadata(a), fs::metadata(b)) else {
        return false;
    };
    if ma.len() != mb.len() {
        return false;
    }
    match (file_hash(a), file_hash(b)) {
        (Some(ha), Some(hb)) => ha == hb,
        _ => false,
    }
}

fn file_hash(path: &Path) -> Option<u64> {
    let f = fs::File::open(path).ok()?;
    let mut r = BufReader::new(f);
    let mut h = DefaultHasher::new();
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = r.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.write(&buf[..n]);
    }
    Some(h.finish())
}
