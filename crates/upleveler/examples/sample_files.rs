//! Writes the invented sample files (a career framework, 1:1s with a lead,
//! notes with a colleague and an old diary) into a folder, to open them or
//! try them in the app:
//!
//!     cargo run --example sample_files -- ~/Desktop/upleveler-samples

#[path = "../tests/samples/mod.rs"]
mod samples;

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "upleveler-samples".into());
    let files = samples::write_all(std::path::Path::new(&dir));
    println!("Wrote:");
    for path in [&files.framework, &files.lead, &files.peer, &files.diary] {
        println!("  {}", path.display());
    }
    println!(
        "\nTry them: /ladder import @{}, then /import @{} (notes about @{}), /import @{} (@{}), /import @{}",
        files.framework.display(),
        files.lead.display(),
        samples::LEAD,
        files.peer.display(),
        samples::PEER,
        files.diary.display()
    );
}
