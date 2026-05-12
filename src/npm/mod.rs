pub mod types;

use percent_encoding::percent_decode_str;

pub fn split_scope_name(fullname: &str) -> (Option<&str>, &str) {
    fullname
        .strip_prefix('@')
        .and_then(|rest| rest.split_once('/').map(|(s, n)| (Some(s), n)))
        .unwrap_or((None, fullname))
}

pub fn decode_fullname(encoded: &str) -> String {
    percent_decode_str(encoded).decode_utf8_lossy().to_string()
}
