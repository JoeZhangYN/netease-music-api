use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::Json;
use futures::stream::Stream;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;

use crate::web::response::APIResponse;
use crate::web::state::AppState;

pub async fn parse_stats(
    State(state): State<Arc<AppState>>,
) -> (StatusCode, Json<APIResponse>) {
    let stats = state.stats.get_all();
    APIResponse::success(stats, "success")
}

pub async fn parse_stats_stream(
    State(state): State<Arc<AppState>>,
) -> Sse<impl Stream<Item = Result<Event, Infallible>>> {
    let rx = state.sse_tx.subscribe();

    let initial_stats = state.stats.get_all();
    let initial_msg = serde_json::to_string(&initial_stats).unwrap_or_default();

    let initial_stream = futures::stream::once(async move {
        Ok::<_, Infallible>(Event::default().data(initial_msg))
    });

    let broadcast_stream = BroadcastStream::new(rx).filter_map(|result: Result<String, _>| {
        match result {
            Ok(msg) => {
                let data = msg
                    .strip_prefix("data: ")
                    .and_then(|s: &str| s.strip_suffix("\n\n"))
                    .unwrap_or(&msg)
                    .to_string();
                Some(Ok::<_, Infallible>(Event::default().data(data)))
            }
            Err(_) => None,
        }
    });

    let stream = initial_stream.chain(broadcast_stream);

    Sse::new(stream).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(30))
            .text("keepalive"),
    )
}
