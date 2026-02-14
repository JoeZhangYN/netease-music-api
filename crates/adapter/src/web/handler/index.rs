use axum::response::Html;

const INDEX_HTML: &str = include_str!("../../../../../templates/index.html");

pub async fn index_handler() -> Html<&'static str> {
    Html(INDEX_HTML)
}
