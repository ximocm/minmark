// Parser benchmark — Phase 0 (ISSUE-010)
// Measure parse performance for 20MB and 100MB documents.

use std::fs;
use std::time::Instant;
use pulldown_cmark::{Parser, Options};

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() != 2 {
        eprintln!("Usage: parser <file.md>");
        std::process::exit(1);
    }

    let path = &args[1];
    let content = fs::read_to_string(path).expect("Failed to read file");

    let mut options = Options::empty();
    options.insert(Options::ENABLE_TABLES);
    options.insert(Options::ENABLE_FOOTNOTES);
    options.insert(Options::ENABLE_STRIKETHROUGH);
    options.insert(Options::ENABLE_TASKLISTS);

    let start = Instant::now();
    let parser = Parser::new_ext(&content, options);
    let events: Vec<_> = parser.collect();
    let duration = start.elapsed();

    println!("File: {}", path);
    println!("Size: {:.2} MB", content.len() as f64 / 1_048_576.0);
    println!("Events: {}", events.len());
    println!("Parse time: {:?}", duration);
}
