use mupdf::{Document, TextPageFlags};
use std::env;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = env::args().nth(1).unwrap_or_else(|| "/home/advprop/pdfs/2510.26692v2-kimi.pdf".to_string());

    let doc = Document::open(&path)?;
    let page = doc.load_page(0)?;
    let text_page = page.to_text_page(TextPageFlags::empty())?;

    for block in text_page.blocks() {
        for line in block.lines() {
            for ch in line.chars() {
                if let Some(c) = ch.char() {
                    print!("{}", c);
                }
            }
            println!();
        }
    }
    Ok(())
}
