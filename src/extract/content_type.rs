pub fn guess(filepath: &str) -> String {
    let mime = mime_guess::from_path(filepath).first_or_octet_stream();
    let essence = mime.essence_str().to_string();
    match essence.as_str() {
        "text/html" | "application/xml" => "text/plain".to_string(),
        _ => essence,
    }
}
