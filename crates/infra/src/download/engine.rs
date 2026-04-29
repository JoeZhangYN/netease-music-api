// file-size-gate: exempt PR-1 (CI bootstrap); PR-8 拆 engine/{ctx,probe,single_stream,ranged,resume,retry}.rs 各 ≤150 SLOC

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use reqwest::Client;
use tokio::sync::Mutex;
use tracing::warn;

use crate::cache::cover_cache::CoverCache;
use netease_domain::model::download::DownloadResult;
use netease_domain::model::music_info::{build_file_path, MusicInfo};
use netease_domain::model::quality::DEFAULT_QUALITY;
use netease_domain::port::cookie_store::CookieStore;
use netease_domain::port::music_api::MusicApi;
use netease_domain::service::download_service;
use netease_kernel::error::AppError;

use super::tags::write_music_tags;

const RETRY_DELAYS_MS: [u64; 5] = [500, 1000, 2000, 4000, 8000];

#[derive(Debug, Clone)]
pub struct DownloadConfig {
    pub ranged_threshold: u64,
    pub ranged_threads: usize,
    pub max_retries: usize,
    pub min_free_disk: u64,
}

impl Default for DownloadConfig {
    fn default() -> Self {
        Self {
            ranged_threshold: 5 * 1024 * 1024,
            ranged_threads: 8,
            max_retries: 5,
            min_free_disk: 500 * 1024 * 1024,
        }
    }
}

pub fn download_client() -> &'static Client {
    static CLIENT: OnceLock<Client> = OnceLock::new();
    CLIENT.get_or_init(|| {
        Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .read_timeout(Duration::from_secs(60))
            .pool_max_idle_per_host(10)
            .pool_idle_timeout(Duration::from_secs(90))
            .build()
            .expect("Failed to create download HTTP client")
    })
}

pub type ProgressCallback = Arc<dyn Fn(u64, u64) + Send + Sync>;

/// Compute the staging `.part` path for a given final destination.
/// Uses `<final_name>.part` to make resumable downloads identifiable
/// (PR-8 will read .part size for Range resume).
pub fn part_path_for(file_path: &Path) -> PathBuf {
    let mut name = file_path
        .file_name()
        .map(|n| n.to_os_string())
        .unwrap_or_default();
    name.push(".part");
    file_path.with_file_name(name)
}

/// Download a file from URL. Uses content_length_hint from API response
/// instead of HEAD request to avoid consuming one-time download links.
/// For large files (>5MB), probes Range support via the first chunk download
/// so zero requests are wasted.
///
/// PR-3 hotfix: writes to `<file>.part` then atomic-renames to final path
/// on success. On failure, the final-name file is never created (so the
/// `cached_size == expected` check in callers can safely treat it as
/// "no cache" instead of treating a truncated file as cached).
pub async fn download_file_ranged(
    _client: &Client,
    url: &str,
    file_path: &Path,
    content_length_hint: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), AppError> {
    let dl = download_client();

    if let Some(parent) = file_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // PR-3: stage to `.part` so that the final-name file only ever exists
    // when fully written. Prevents truncated files being treated as cached.
    let part_path = part_path_for(file_path);
    let content_length = content_length_hint;

    let result = if content_length > config.ranged_threshold {
        download_adaptive(
            dl,
            url,
            &part_path,
            content_length,
            on_progress.clone(),
            config,
        )
        .await
    } else {
        download_single_stream(
            dl,
            url,
            &part_path,
            content_length,
            on_progress,
            config.max_retries,
        )
        .await
    };

    match result {
        Ok(()) => {
            // PR-3: atomic rename .part → final on success.
            // If .part exists at expected size, this is the only atomic
            // commit point — partial writes never carry the final name.
            tokio::fs::rename(&part_path, file_path)
                .await
                .map_err(|e| {
                    AppError::Download(format!(
                        "Rename .part to final failed ({}): {}",
                        file_path.display(),
                        e
                    ))
                })?;
            Ok(())
        }
        Err(e) => {
            // PR-3: keep .part on failure so PR-8 (engine FSM) can resume
            // from existing offset. The final-name file was never written,
            // so callers' cached_size check stays correct.
            Err(e)
        }
    }
}

/// For large files: first Range GET doubles as probe and first chunk download.
/// If 206 → Range supported, download remaining chunks in parallel.
/// If 200 → Range not supported, stream this response directly (no wasted request).
async fn download_adaptive(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<(), AppError> {
    let ranged_threads = config.ranged_threads;
    let max_retries = config.max_retries;
    let chunk_size = content_length / ranged_threads as u64;
    let first_end = chunk_size - 1;

    let resp = match client
        .get(url)
        .header("Range", format!("bytes=0-{}", first_end))
        .send()
        .await
    {
        Ok(r) => r,
        Err(_) => {
            return download_single_stream(
                client,
                url,
                file_path,
                content_length,
                on_progress,
                max_retries,
            )
            .await;
        }
    };

    let status = resp.status().as_u16();

    if status == 206 {
        let first_data = resp
            .bytes()
            .await
            .map_err(|e| AppError::Download(format!("Read first chunk: {}", e)))?
            .to_vec();

        if let Some(ref cb) = on_progress {
            cb(first_data.len() as u64, content_length);
        }

        download_remaining_and_assemble(
            client,
            url,
            file_path,
            content_length,
            first_data,
            chunk_size,
            ranged_threads,
            max_retries,
            on_progress,
        )
        .await
    } else if status == 200 || status == 203 {
        stream_response_to_file(resp, file_path, content_length, on_progress).await
    } else {
        download_single_stream(
            client,
            url,
            file_path,
            content_length,
            on_progress,
            max_retries,
        )
        .await
    }
}

/// Stream an already-opened response to a file.
async fn stream_response_to_file(
    resp: reqwest::Response,
    file_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
) -> Result<(), AppError> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(content_length);

    let mut file = tokio::fs::File::create(file_path)
        .await
        .map_err(|e| AppError::Download(format!("Create file failed: {}", e)))?;

    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::Download(format!("Stream error: {}", e)))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| AppError::Download(format!("Write error: {}", e)))?;
        downloaded += chunk.len() as u64;
        if let Some(ref cb) = on_progress {
            if total > 0 {
                cb(downloaded, total);
            }
        }
    }

    file.flush()
        .await
        .map_err(|e| AppError::Download(format!("Flush error: {}", e)))?;

    // PR-3: see download_stream_once for rationale.
    if total > 0 && downloaded != total {
        return Err(AppError::Download(format!(
            "Stream short read (probe-response path): got {} of {} bytes",
            downloaded, total
        )));
    }

    Ok(())
}

/// Download remaining chunks (2..N) in parallel, then assemble with first chunk.
#[allow(clippy::too_many_arguments)]
async fn download_remaining_and_assemble(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    first_data: Vec<u8>,
    chunk_size: u64,
    ranged_threads: usize,
    max_retries: usize,
    on_progress: Option<ProgressCallback>,
) -> Result<(), AppError> {
    let downloaded_total = Arc::new(std::sync::atomic::AtomicU64::new(first_data.len() as u64));
    let results: Arc<Mutex<HashMap<u64, Vec<u8>>>> = Arc::new(Mutex::new(HashMap::new()));
    results.lock().await.insert(0, first_data);

    let mut ranges = Vec::new();
    for i in 1..ranged_threads {
        let start = i as u64 * chunk_size;
        let end = if i == ranged_threads - 1 {
            content_length - 1
        } else {
            (i as u64 + 1) * chunk_size - 1
        };
        ranges.push((start, end));
    }

    let mut handles = Vec::new();
    for (start, end) in ranges.clone() {
        let client = client.clone();
        let url = url.to_string();
        let downloaded_total = downloaded_total.clone();
        let on_progress = on_progress.clone();
        let results = results.clone();
        let cl = content_length;

        handles.push(tokio::spawn(async move {
            let expected_len = end - start + 1;
            for attempt in 0..max_retries {
                match fetch_range(&client, &url, start, end).await {
                    Ok(data) => {
                        let actual_len = data.len() as u64;
                        // PR-3: chunk length validation. Short reads (server
                        // returned fewer bytes than the Range requested) must
                        // not silently produce a truncated assembled file.
                        if actual_len != expected_len {
                            warn!(
                                "Range chunk short read: expected {} bytes [{}..{}], got {} (attempt {}/{})",
                                expected_len, start, end, actual_len, attempt + 1, max_retries
                            );
                            if attempt < max_retries - 1 {
                                let delay_idx = attempt.min(RETRY_DELAYS_MS.len() - 1);
                                tokio::time::sleep(Duration::from_millis(
                                    RETRY_DELAYS_MS[delay_idx],
                                ))
                                .await;
                                continue;
                            }
                            return Err(AppError::Download(format!(
                                "Range chunk short read after {} retries: \
                                 expected {} bytes [{}..{}], got {}",
                                max_retries, expected_len, start, end, actual_len
                            )));
                        }

                        downloaded_total.fetch_add(actual_len, std::sync::atomic::Ordering::Relaxed);
                        if let Some(ref cb) = on_progress {
                            cb(
                                downloaded_total.load(std::sync::atomic::Ordering::Relaxed),
                                cl,
                            );
                        }
                        results.lock().await.insert(start, data);
                        return Ok(());
                    }
                    Err(e) => {
                        if attempt < max_retries - 1 {
                            let delay_idx = attempt.min(RETRY_DELAYS_MS.len() - 1);
                            tokio::time::sleep(Duration::from_millis(RETRY_DELAYS_MS[delay_idx]))
                                .await;
                            continue;
                        }
                        return Err(e);
                    }
                }
            }
            Err(AppError::Download(
                "Range download failed after retries".into(),
            ))
        }));
    }

    for handle in handles {
        handle
            .await
            .map_err(|e| AppError::Download(format!("Task join error: {}", e)))?
            .map_err(|e| AppError::Download(format!("Range download failed: {}", e)))?;
    }

    let chunks = results.lock().await;
    let mut file = std::fs::File::create(file_path)
        .map_err(|e| AppError::Download(format!("Create file failed: {}", e)))?;

    use std::io::Write;

    // Write first chunk
    if let Some(data) = chunks.get(&0) {
        file.write_all(data)
            .map_err(|e| AppError::Download(format!("Write error: {}", e)))?;
    }

    // Write remaining chunks in order
    for (start, _) in &ranges {
        if let Some(data) = chunks.get(start) {
            file.write_all(data)
                .map_err(|e| AppError::Download(format!("Write error: {}", e)))?;
        }
    }
    drop(file);

    // PR-3: post-assembly size verification — any chunk-write skip or first
    // chunk short-read that earlier checks missed gets caught here.
    let written = std::fs::metadata(file_path).map(|m| m.len()).unwrap_or(0);
    if written != content_length {
        return Err(AppError::Download(format!(
            "Assembled file size mismatch: wrote {} bytes, expected {} ({})",
            written,
            content_length,
            file_path.display()
        )));
    }

    Ok(())
}

async fn download_single_stream(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    on_progress: Option<ProgressCallback>,
    max_retries: usize,
) -> Result<(), AppError> {
    for attempt in 0..max_retries {
        match download_stream_once(client, url, file_path, content_length, &on_progress).await {
            Ok(()) => return Ok(()),
            Err(e) => {
                if attempt < max_retries - 1 {
                    warn!("Download attempt {} failed: {} - retrying", attempt + 1, e);
                    let delay_idx = attempt.min(RETRY_DELAYS_MS.len() - 1);
                    tokio::time::sleep(Duration::from_millis(RETRY_DELAYS_MS[delay_idx])).await;
                    continue;
                }
                return Err(e);
            }
        }
    }
    unreachable!()
}

async fn download_stream_once(
    client: &Client,
    url: &str,
    file_path: &Path,
    content_length: u64,
    on_progress: &Option<ProgressCallback>,
) -> Result<(), AppError> {
    use futures::StreamExt;
    use tokio::io::AsyncWriteExt;

    let resp = client
        .get(url)
        .send()
        .await
        .map_err(|e| AppError::Download(format!("Download request failed: {}", e)))?;

    // PR-5: reqwest does NOT error on HTTP 4xx/5xx — it only errors on
    // transport failures. Without this guard, an empty 500 body
    // (content-length: 0) silently passes the size-mismatch check
    // and gets renamed to the final path. Reject non-success here.
    let status = resp.status();
    if !status.is_success() && status.as_u16() != 206 {
        return Err(AppError::Download(format!(
            "HTTP {} from server",
            status.as_u16()
        )));
    }

    let total = resp
        .headers()
        .get("content-length")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.parse::<u64>().ok())
        .unwrap_or(content_length);

    let mut file = tokio::fs::File::create(file_path)
        .await
        .map_err(|e| AppError::Download(format!("Create file failed: {}", e)))?;

    let mut downloaded: u64 = 0;
    let mut stream = resp.bytes_stream();

    while let Some(chunk) = stream.next().await {
        let chunk = chunk.map_err(|e| AppError::Download(format!("Stream error: {}", e)))?;
        file.write_all(&chunk)
            .await
            .map_err(|e| AppError::Download(format!("Write error: {}", e)))?;
        downloaded += chunk.len() as u64;
        if let Some(ref cb) = on_progress {
            if total > 0 {
                cb(downloaded, total);
            }
        }
    }

    file.flush()
        .await
        .map_err(|e| AppError::Download(format!("Flush error: {}", e)))?;

    // PR-3: stream short-read detection. Server can close mid-transfer; an
    // unconditional Ok would leave a truncated file with the .part name,
    // and the rename + cached_size check would later read it as cached
    // unless we error here.
    if total > 0 && downloaded != total {
        return Err(AppError::Download(format!(
            "Stream short read: got {} of {} bytes",
            downloaded, total
        )));
    }

    Ok(())
}

async fn fetch_range(
    client: &Client,
    url: &str,
    start: u64,
    end: u64,
) -> Result<Vec<u8>, AppError> {
    let resp = client
        .get(url)
        .header("Range", format!("bytes={}-{}", start, end))
        .send()
        .await
        .map_err(|e| AppError::Download(format!("Range request failed: {}", e)))?;

    if resp.status().as_u16() == 503 {
        return Err(AppError::Download("Server returned 503".into()));
    }

    let data = resp
        .bytes()
        .await
        .map_err(|e| AppError::Download(format!("Read range bytes failed: {}", e)))?;

    Ok(data.to_vec())
}

#[allow(clippy::too_many_arguments)] // PR-1 scope: bootstrap CI; PR-8 拆 DownloadCtx struct 时根除
pub async fn download_music_file(
    client: &Client,
    api: &dyn MusicApi,
    cookie_store: &dyn CookieStore,
    cover_cache: &CoverCache,
    downloads_dir: &Path,
    music_id: &str,
    quality: &str,
    on_progress: Option<ProgressCallback>,
    config: &DownloadConfig,
) -> Result<DownloadResult, AppError> {
    let cookies = cookie_store.parse().unwrap_or_default();
    let music_info = download_service::get_music_info(api, music_id, quality, &cookies).await?;
    let file_path = build_file_path(downloads_dir, &music_info, quality);

    if let Some(parent) = file_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // PR-3: only treat as cached if size matches expected exactly.
    // Previously `cached_size > 0` accepted any non-zero file as complete,
    // which masked truncated files left by interrupted prior downloads.
    let cached_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
    if cached_size > 0 && music_info.file_size > 0 && cached_size == music_info.file_size {
        let cover_data = cover_cache.fetch(client, &music_info.pic_url).await;
        return Ok(DownloadResult::ok_with_cover(
            file_path,
            cached_size,
            music_info,
            cover_data,
        ));
    }
    if cached_size > 0 && cached_size != music_info.file_size {
        // Truncated/oversized leftover from a prior failed run — remove
        // before re-downloading so the .part rename succeeds atomically.
        warn!(
            "Removing truncated cached file {} ({}B vs expected {}B)",
            file_path.display(),
            cached_size,
            music_info.file_size
        );
        let _ = std::fs::remove_file(&file_path);
    }

    super::disk_guard::ensure_disk_space(
        downloads_dir,
        music_info.file_size,
        config.min_free_disk,
    )?;

    let (dl_result, cover_data) = tokio::join!(
        download_file_ranged(
            client,
            music_info.download_url.as_str(),
            &file_path,
            music_info.file_size,
            on_progress,
            config
        ),
        cover_cache.fetch(client, &music_info.pic_url),
    );
    dl_result?;

    write_music_tags(&file_path, &music_info, cover_data.as_deref());

    let size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
    Ok(DownloadResult::ok_with_cover(
        file_path, size, music_info, cover_data,
    ))
}

pub async fn download_music_with_metadata(
    client: &Client,
    downloads_dir: &Path,
    music_info: &MusicInfo,
    cover_data: Option<&[u8]>,
    on_progress: Option<ProgressCallback>,
    do_write_tags: bool,
    config: &DownloadConfig,
) -> Result<DownloadResult, AppError> {
    let quality = if music_info.quality.is_empty() {
        DEFAULT_QUALITY
    } else {
        &music_info.quality
    };
    let file_path = build_file_path(downloads_dir, music_info, quality);

    if let Some(parent) = file_path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }

    // PR-3: exact size match cache check (see download_music_file comment).
    let cached_size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
    if cached_size > 0 && music_info.file_size > 0 && cached_size == music_info.file_size {
        return Ok(DownloadResult::ok(
            file_path,
            cached_size,
            music_info.clone(),
        ));
    }
    if cached_size > 0 && cached_size != music_info.file_size {
        warn!(
            "Removing truncated cached file {} ({}B vs expected {}B)",
            file_path.display(),
            cached_size,
            music_info.file_size
        );
        let _ = std::fs::remove_file(&file_path);
    }

    super::disk_guard::ensure_disk_space(
        downloads_dir,
        music_info.file_size,
        config.min_free_disk,
    )?;

    download_file_ranged(
        client,
        music_info.download_url.as_str(),
        &file_path,
        music_info.file_size,
        on_progress,
        config,
    )
    .await?;

    if do_write_tags {
        write_music_tags(&file_path, music_info, cover_data);
    }

    let size = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
    Ok(DownloadResult::ok(file_path, size, music_info.clone()))
}
