use crate::model::cookie::{is_cookies_valid, parse_cookie_string};
use crate::port::cookie_store::CookieStore;
use netease_kernel::error::AppError;

pub fn validate_and_save(store: &dyn CookieStore, raw_cookie: &str) -> Result<bool, AppError> {
    let cookies = parse_cookie_string(raw_cookie);
    let valid = is_cookies_valid(&cookies);
    store.write(raw_cookie)?;
    Ok(valid)
}

pub fn check_status(store: &dyn CookieStore) -> bool {
    store.is_valid()
}
