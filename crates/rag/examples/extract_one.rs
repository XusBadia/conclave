//! Diagnostic harness: run the extraction pipeline on a single file and
//! report timing. Used to reproduce extractor hangs on problem PDFs:
//!
//! ```sh
//! cargo run -p conclave-rag --example extract_one -- /path/to/file.pdf
//! ```

fn main() {
    let path = std::env::args().nth(1).expect("usage: extract_one <file>");
    let started = std::time::Instant::now();
    eprintln!("extracting {path} …");
    match conclave_rag::extract_from_path(std::path::Path::new(&path)) {
        Ok(t) => println!(
            "ok: {} chars, needs_ocr={}, elapsed={:?}",
            t.content.len(),
            t.needs_ocr,
            started.elapsed()
        ),
        Err(e) => println!("err: {e} (elapsed={:?})", started.elapsed()),
    }
}
