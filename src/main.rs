use anyhow::Result;

fn main() -> Result<()> {
    println!("qbook {}", env!("CARGO_PKG_VERSION"));
    Ok(())
}
