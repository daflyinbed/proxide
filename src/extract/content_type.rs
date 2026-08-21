const TEXT_EXTENSIONS: &[&str] = &[
    "ts", "tsx", "mts", "cts", "mjs", "cjs", "jsx", "map", "vue", "svelte", "astro",
];

const BINARY_EXTENSIONS: &[&str] = &[
    "png", "jpg", "jpeg", "gif", "webp", "bmp", "ico", "tif", "tiff", "avif", "heic", "woff",
    "woff2", "ttf", "otf", "eot", "mp3", "mp4", "webm", "ogg", "ogv", "wav", "flac", "aac",
    "avi", "mov", "mkv", "zip", "gz", "tgz", "bz2", "xz", "7z", "rar", "zst", "br", "tar",
    "wasm", "pdf", "swf", "class", "jar", "exe", "dll", "so", "dylib", "node", "db", "sqlite",
];

pub fn guess(filepath: &str) -> String {
    let mime = mime_guess::from_path(filepath).first_or_octet_stream();
    let essence = mime.essence_str().to_string();
    match essence.as_str() {
        "text/html" | "application/xml" => "text/plain".to_string(),
        _ => essence,
    }
}

pub fn is_compressible(filepath: &str, content_type: &str) -> bool {
    let ext = filepath.rsplit('.').next().unwrap_or("");
    if TEXT_EXTENSIONS.contains(&ext) {
        return true;
    }
    if BINARY_EXTENSIONS.contains(&ext) {
        return false;
    }
    let binary_type = (content_type.starts_with("image/") && content_type != "image/svg+xml")
        || content_type.starts_with("video/")
        || content_type.starts_with("audio/")
        || content_type.starts_with("font/")
        || matches!(
            content_type,
            "application/zip"
                | "application/x-tar"
                | "application/gzip"
                | "application/x-gzip"
                | "application/x-bzip2"
                | "application/x-xz"
                | "application/x-7z-compressed"
                | "application/x-rar-compressed"
                | "application/wasm"
                | "application/pdf"
                | "application/octet-stream"
        );
    !binary_type
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn is_compressible_classifies_text_and_binary() {
        assert!(is_compressible("lib/index.js", "text/javascript"));
        assert!(is_compressible("pkg/package.json", "application/json"));
        assert!(is_compressible("README.md", "text/markdown"));
        assert!(is_compressible("src/mod.ts", "video/mp2t"));
        assert!(is_compressible("src/app.mjs", "application/octet-stream"));
        assert!(is_compressible("assets/icon.svg", "image/svg+xml"));
        assert!(!is_compressible("assets/logo.png", "image/png"));
        assert!(!is_compressible("fonts/main.woff2", "font/woff2"));
        assert!(!is_compressible("lib/binding.node", "application/octet-stream"));
        assert!(!is_compressible("wasm/index.wasm", "application/wasm"));
        assert!(!is_compressible("unknown.bin", "application/octet-stream"));
    }
}
