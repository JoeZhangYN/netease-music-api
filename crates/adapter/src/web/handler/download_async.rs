// file-size-gate: exempt PR-1 (CI bootstrap); PR-9 handler 拆 download_async/{start,progress,result}.rs，worker 上移 domain::service::download_service::execute_single

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use axum::body::Body;
use axum::extract::{Path, State};
use axum::http::{header, StatusCode};
use axum::response::{IntoResponse, Response};
use axum::Json;
use serde::Deserialize;
use serde_json::json;
use tracing::error;

use crate::web::helpers::{build_url_refresher, PermitGuard, StatsKind};
use crate::web::response::APIResponse;
use crate::web::state::AppState;
use netease_domain::model::download::TaskStage;
use netease_domain::model::music_info::{DownloadUrl, MusicInfo};
use netease_domain::model::quality::DEFAULT_QUALITY;
use netease_domain::service::download_service;
use netease_infra::download::engine::{
    download_music_with_metadata, DownloadConfig, ProgressCallback,
};
use netease_infra::download::tags::write_music_tags_async;
use netease_infra::download::zip::{build_zip_to_file, TrackData};
use netease_infra::extract_id::extract_music_id;
use netease_kernel::error::AppError;

#[derive(Debug, Deserialize)]
pub struct DownloadStartRequest {
    pub id: Option<serde_json::Value>,
    pub quality: Option<String>,
    pub name: Option<String>,
    pub artists: Option<String>,
    pub album: Option<String>,
    pub pic_url: Option<String>,
    pub lyric: Option<String>,
    pub tlyric: Option<String>,
}

pub async fn download_start(
    State(state): State<Arc<AppState>>,
    Json(data): Json<DownloadStartRequest>,
) -> (StatusCode, Json<APIResponse>) {
    let raw_id = match &data.id {
        Some(serde_json::Value::String(s)) => s.clone(),
        Some(serde_json::Value::Number(n)) => n.to_string(),
        _ => return APIResponse::error("缺少参数 'id'", 400),
    };

    let quality = data
        .quality
        .clone()
        .unwrap_or_else(|| DEFAULT_QUALITY.into());
    let music_id = extract_music_id(&raw_id, &state.http_client).await;
    let dedup_key = format!("{music_id}_{quality}");

    if let Some(existing) = state.dedup.get(&dedup_key) {
        let existing_task_id = existing.value().clone();
        if let Some(task) = state.task_store.get(&existing_task_id) {
            if task.stage.is_reusable_for_dedup() {
                return APIResponse::success(
                    json!({"task_id": existing_task_id}),
                    "已有相同下载任务，正在复用",
                );
            }
        }
    }

    let metadata = (data.name.is_some() && data.artists.is_some()).then(|| MusicInfo {
        id: music_id.parse().unwrap_or(0),
        name: data.name.unwrap_or_default(),
        artists: data.artists.unwrap_or_default(),
        album: data.album.unwrap_or_default(),
        pic_url: data.pic_url.unwrap_or_default(),
        duration: 0,
        track_number: 0,
        download_url: DownloadUrl::new(String::new()),
        file_type: String::new(),
        file_size: 0,
        quality: quality.clone(),
        lyric: data.lyric.unwrap_or_default(),
        tlyric: data.tlyric.unwrap_or_default(),
    });

    let task_id = state.task_store.create();
    state.dedup.insert(dedup_key.clone(), task_id.clone());

    let state_clone = Arc::clone(&state);
    let task_id_clone = task_id.clone();
    let music_id_clone = music_id;
    let quality_clone = quality.clone();
    let dedup_key_clone = dedup_key;
    tokio::spawn(async move {
        single_download_worker(
            state_clone,
            task_id_clone,
            music_id_clone,
            quality_clone,
            metadata,
            Some(dedup_key_clone),
        )
        .await;
    });

    APIResponse::success(json!({"task_id": task_id}), "下载任务已启动")
}

pub async fn download_cancel(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> (StatusCode, Json<APIResponse>) {
    state.cancelled.insert(task_id.clone(), ());
    state.task_store.update(
        &task_id,
        Box::new(|t| {
            t.stage = TaskStage::Error;
            t.error = Some("已取消".into());
        }),
    );
    APIResponse::success(json!({"task_id": task_id}), "任务已取消")
}

pub async fn download_progress(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let Some(task) = state.task_store.get(&task_id) else {
        return APIResponse::error("任务不存在或已过期", 404).into_response();
    };

    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let resp_data = json!({
        "stage": task.stage,
        "percent": task.percent,
        "detail": task.detail,
        "error": task.error,
        "current": task.current,
        "total": task.total,
        "completed": task.completed,
        "failed": task.failed,
        "elapsed": now.saturating_sub(task.created_at),
    });

    let (status, json) = APIResponse::success(resp_data, "success");
    let mut response = (status, json).into_response();
    // "no-store" 是 RFC 7234 静态合法 header value，parse 必成功
    #[allow(clippy::unwrap_used)]
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, "no-store".parse().unwrap());
    response
}

pub async fn download_result(
    State(state): State<Arc<AppState>>,
    Path(task_id): Path<String>,
) -> Response {
    let Some(task) = state.task_store.get(&task_id) else {
        return APIResponse::error("任务不存在或已过期", 404).into_response();
    };

    if !task.stage.is_downloadable_to_user() {
        return APIResponse::error("任务尚未完成", 400).into_response();
    }

    let first_access = task.stage == TaskStage::Done;
    if first_access {
        state.task_store.update(
            &task_id,
            Box::new(|t| {
                t.stage = TaskStage::Retrieved;
            }),
        );
    }

    let zip_path = match &task.zip_path {
        Some(p) => p.clone(),
        None => return APIResponse::error("文件数据丢失", 500).into_response(),
    };

    let zip_filename = task.zip_filename.unwrap_or_else(|| "download.zip".into());

    let file_size = match tokio::fs::metadata(&zip_path).await {
        Ok(m) => m.len(),
        Err(_) => return APIResponse::error("文件数据丢失", 500).into_response(),
    };

    let Ok(file) = tokio::fs::File::open(&zip_path).await else {
        return APIResponse::error("文件数据丢失", 500).into_response();
    };

    let encoded_fn = urlencoding::encode(&zip_filename);
    let zip_path_owned = zip_path.clone();

    let stream = tokio_util::io::ReaderStream::new(file);
    let body = Body::from_stream(stream);

    if first_access {
        tokio::spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(300)).await;
            // destructive-audit: exempt — 5min 后清理临时 zip，fire-and-forget 合理
            let _ = tokio::fs::remove_file(&zip_path_owned).await;
        });
    }

    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "application/octet-stream")
        .header(
            header::CONTENT_DISPOSITION,
            format!("attachment; filename*=UTF-8''{encoded_fn}"),
        )
        .header("X-Download-Filename", encoded_fn.as_ref())
        .header(header::CONTENT_LENGTH, file_size.to_string())
        .body(body)
        .ok()
        .unwrap_or_else(|| APIResponse::error("Response build failed", 500).into_response())
}

/// 单曲后台下载 worker（`do_single_download`）的 typed 失败分类，取代 pre-v4 的
/// `Result<(), String>`（同步删除其 `grep-gate-skip` 豁免注释 + `#[rustfmt::skip]`）。
///
/// boundary `single_download_worker` 用 `Display` 渲染 `t.error` 文案——**用户面
/// 错误文案的唯一来源**（SSOT：前端轮询 `task.error` 语义在此集中，取代散落在
/// `do_single_download` 体内的 3 处 `task_store.update(stage=Error, error=…)`）。
///
/// 不经 `AppErrorResponse`（不变量 #7 的 HTTP 状态映射）：背景 worker 写
/// `t.error: Option<String>` 供前端轮询、非 HTTP 响应，故用户文案留在 adapter
/// 边界，域层 `DownloadError` / kernel `AppError` 保持纯净（AP-007）。`Display`
/// 走**显式穷尽 match**（无 `_` 兜底）——新增变体不补文案即编译错（反退化锁）。
#[derive(Debug)]
enum SingleDownloadError {
    /// 上游 typed 错误透传（`get_song_url` / `get_music_info` / 下载引擎 /
    /// 信号量关闭）——复用内部 `AppError` 文案，保持 typed 升级前语义。
    Upstream(AppError),
    /// 网易云返回空下载链接（无版权 / 需购买）。
    NoUrl,
    /// 外层 per-song 超时——`.part` 留盘，重试从断点续传（不变量 #1/#22）。
    Timeout { secs: u64 },
    /// ZIP 打包失败。
    Packaging(String),
}

impl std::fmt::Display for SingleDownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Upstream(e) => write!(f, "{e}"),
            Self::NoUrl => write!(f, "无可用的下载链接"),
            Self::Timeout { secs } => {
                write!(
                    f,
                    "下载超时（{secs}秒）。已下载部分保留，重试将从断点续传。"
                )
            }
            Self::Packaging(msg) => write!(f, "{msg}"),
        }
    }
}

impl From<AppError> for SingleDownloadError {
    fn from(e: AppError) -> Self {
        Self::Upstream(e)
    }
}

async fn single_download_worker(
    state: Arc<AppState>,
    task_id: String,
    music_id: String,
    quality: String,
    metadata: Option<MusicInfo>,
    dedup_key: Option<String>,
) {
    let Ok(_download_guard) = PermitGuard::acquire(
        Arc::clone(&state.download_semaphore),
        Arc::clone(&state.stats),
        StatsKind::Download,
        std::time::Duration::from_secs(60),
    )
    .await
    else {
        state.task_store.update(
            &task_id,
            Box::new(|t| {
                t.stage = TaskStage::Error;
                t.error = Some("下载队列繁忙，请稍后重试".into());
            }),
        );
        if let Some(ref key) = dedup_key {
            state.dedup.remove(key);
        }
        return;
    };

    if let Err(e) = do_single_download(&state, &task_id, &music_id, &quality, metadata).await {
        error!("Background download error: {}", e);
        // 单一渲染点：typed 错误 → t.error 文案（SSOT，见 SingleDownloadError 文档）。
        let msg = e.to_string();
        state.task_store.update(
            &task_id,
            Box::new(move |t| {
                t.stage = TaskStage::Error;
                t.error = Some(msg);
            }),
        );
    }

    if let Some(ref key) = dedup_key {
        state.dedup.remove(key);
    }
}

async fn do_single_download(
    state: &AppState,
    task_id: &str,
    music_id: &str,
    quality: &str,
    metadata: Option<MusicInfo>,
) -> Result<(), SingleDownloadError> {
    state.task_store.update(
        task_id,
        Box::new(|t| {
            t.stage = TaskStage::FetchingUrl;
            t.percent = 0;
            t.detail = "正在获取下载链接...".into();
        }),
    );

    let cookies = state.cookie_store.parse().unwrap_or_default();
    let client = &state.http_client;
    let api = state.music_api.as_ref();
    let fallback_cfg =
        netease_domain::service::song_service::QualityFallbackConfig::from_runtime_config(
            &state.runtime_config.load(),
        );
    // PR-E: 下载侧 CDN 速率护栏（共享 limiter，host=cdn 与 API 域分桶）
    let cdn_key = netease_infra::http::RateLimitKey {
        host: "cdn".into(),
        user: netease_infra::http::extract_user_key(&cookies),
    };
    let _ = state.rate_limiter.acquire(&cdn_key).await;

    let music_info = if let Some(mut meta) = metadata {
        // PR-6: get_song_url returns typed SongUrlData; no more .pointer()
        // typed 升级：AppError 经 `?` → SingleDownloadError::Upstream（透传文案）。
        let url_data = api.get_song_url(music_id, quality, &cookies).await?;

        if url_data.url.is_empty() {
            return Err(SingleDownloadError::NoUrl);
        }

        meta.download_url = DownloadUrl::new(url_data.url);
        meta.file_type = url_data.file_type;
        meta.file_size = url_data.size;
        meta
    } else {
        state.task_store.update(
            task_id,
            Box::new(|t| {
                t.detail = "正在获取歌曲信息...".into();
            }),
        );

        // 背景任务原语义：无超时永久等 parse 许可（仅 semaphore closed 才 Err）。
        // acquire_unbounded 保留该行为 + RAII 兜 panic。
        let parse_guard = PermitGuard::acquire_unbounded(
            Arc::clone(&state.parse_semaphore),
            Arc::clone(&state.stats),
            StatsKind::Parse,
        )
        .await?;

        let info_result = download_service::get_music_info(
            api,
            music_id,
            quality,
            &cookies,
            &fallback_cfg,
            task_id,
        )
        .await;

        drop(parse_guard);

        info_result?
    };

    if state.cancelled.remove(task_id).is_some() {
        return Ok(());
    }

    let cover_future = {
        let client = client.clone();
        let pic_url = music_info.pic_url.clone();
        let cache = Arc::clone(&state.cover_cache);
        tokio::spawn(async move {
            if pic_url.is_empty() {
                None
            } else {
                cache.fetch(&client, &pic_url).await
            }
        })
    };

    state.task_store.update(
        task_id,
        Box::new(|t| {
            t.stage = TaskStage::Downloading;
            t.percent = 5;
            t.detail = "正在下载音乐文件 (0%)...".into();
        }),
    );

    let task_id_owned = task_id.to_string();
    let task_store = Arc::clone(&state.task_store);
    let progress_cb: ProgressCallback = Arc::new(move |downloaded, total| {
        if total > 0 {
            let pct = 5 + (downloaded as f64 / total as f64 * 85.0) as u32;
            let file_pct = (downloaded as f64 / total as f64 * 100.0) as u32;
            let detail = format!("正在下载音乐文件 ({file_pct}%)...");
            task_store.update(
                &task_id_owned,
                Box::new(move |t| {
                    t.percent = pct;
                    t.detail = detail;
                }),
            );
        }
    });

    let (dl_config, dl_timeout_secs) = {
        let rc = state.runtime_config.load();
        // PR-R4: per-song refresher pin 到**实际生效** quality（music_info.quality，
        // #14——非 requested），链接过期时取同 quality 新 URL 续传。
        let mut cfg = DownloadConfig::from_runtime_config(&rc, Arc::clone(&state.in_flight));
        cfg.refresher = Some(build_url_refresher(state, &music_info, &cookies, task_id));
        (cfg, rc.download_timeout_per_song_secs)
    };

    // PR-3: outer timeout — single-song download must fail fast instead of
    // hanging forever when CDN URLs expire mid-stream. Matches the existing
    // batch path's per-song timeout (download_batch.rs).
    let dl_future = download_music_with_metadata(
        client,
        &state.config.downloads_dir,
        &music_info,
        None,
        Some(progress_cb),
        false,
        &dl_config,
    );
    let result = match tokio::time::timeout(
        std::time::Duration::from_secs(dl_timeout_secs),
        dl_future,
    )
    .await
    {
        Ok(Ok(r)) => r,
        Ok(Err(e)) => return Err(e.into()),
        // PR-R4：断点续传 FSM 已落地——超时留下的 `.part` + sidecar manifest 在重试时
        // 被复用（single_stream 按 `.part` 长度续 / ranged 按 manifest 跳已填 chunk），
        // 文案据此成真。复用前提：resume_enabled=true（默认）。文案由 boundary 渲染。
        Err(_) => {
            return Err(SingleDownloadError::Timeout {
                secs: dl_timeout_secs,
            })
        }
    };

    let cover_data = cover_future.await.unwrap_or(None);

    if state.cancelled.remove(task_id).is_some() {
        return Ok(());
    }

    state.task_store.update(
        task_id,
        Box::new(|t| {
            t.stage = TaskStage::Packaging;
            t.percent = 92;
            t.detail = "正在打包...".into();
        }),
    );

    // DownloadOutcome 必填字段：成功必有 file_path（类型不变量）——无需约定式 unwrap。
    let file_path = &result.file_path;
    write_music_tags_async(file_path, &music_info, cover_data.as_deref()).await;

    let zip_dir = std::env::temp_dir().join("music_api_zips");
    // fire-and-forget：dir 已存在 / 权限不足时下游 build_zip_to_file 报告
    let _: std::io::Result<()> = std::fs::create_dir_all(&zip_dir);
    let zip_path = zip_dir.join(format!("{task_id}.zip"));

    let tracks = vec![TrackData {
        file_path: file_path.clone(),
        music_info: music_info.clone(),
        cover_data,
    }];
    build_zip_to_file(&tracks, &zip_path)
        .map_err(|e| SingleDownloadError::Packaging(e.to_string()))?;

    let zip_filename = format!(
        "{}.zip",
        file_path
            .file_stem()
            .and_then(|n| n.to_str())
            .unwrap_or("download")
    );

    let zip_path_str = zip_path.to_string_lossy().to_string();
    state.task_store.update(
        task_id,
        Box::new(move |t| {
            t.stage = TaskStage::Done;
            t.percent = 100;
            t.detail = "下载完成".into();
            t.zip_path = Some(zip_path_str);
            t.zip_filename = Some(zip_filename);
        }),
    );

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    // 反退化锁（SSOT 节4）：用户面 `task.error` 文案锚定在 SingleDownloadError::Display。
    // 任一文案漂移 / 新增变体漏补文案 → 编译错（穷尽 match）或测试红。

    #[test]
    fn no_url_renders_user_facing_message() {
        assert_eq!(SingleDownloadError::NoUrl.to_string(), "无可用的下载链接");
    }

    #[test]
    fn timeout_renders_resume_hint() {
        let e = SingleDownloadError::Timeout { secs: 300 };
        assert_eq!(
            e.to_string(),
            "下载超时（300秒）。已下载部分保留，重试将从断点续传。"
        );
    }

    #[test]
    fn upstream_passes_through_inner_app_error_display() {
        // `?` 透传：AppError → Upstream，文案保持 typed 升级前语义（pre-v4 `e.to_string()`）。
        let e: SingleDownloadError = AppError::AuthExpired.into();
        assert_eq!(e.to_string(), AppError::AuthExpired.to_string());
    }

    #[test]
    fn packaging_passes_through_raw_message() {
        let e = SingleDownloadError::Packaging("zip boom".into());
        assert_eq!(e.to_string(), "zip boom");
    }
}
