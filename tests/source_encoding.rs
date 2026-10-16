use std::path::Path;

fn scan_rust_sources(dir: &Path, bad: &mut Vec<String>) {
    for entry in std::fs::read_dir(dir).expect("read source directory") {
        let path = entry.expect("source entry").path();
        if path.is_dir() {
            scan_rust_sources(&path, bad);
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let source = std::fs::read_to_string(&path).expect("Rust source must be UTF-8");
            for pattern in ["â€", "Â·", "Ã"] {
                if source.contains(pattern) {
                    bad.push(format!("{} contains {pattern}", path.display()));
                }
            }
        }
    }
}

#[test]
fn rust_sources_have_no_common_mojibake() {
    let mut bad = Vec::new();
    scan_rust_sources(&Path::new(env!("CARGO_MANIFEST_DIR")).join("src"), &mut bad);
    assert!(bad.is_empty(), "{}", bad.join("\n"));
}
