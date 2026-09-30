//! Draw every Killchain screen to a PNG over a sample backdrop (no window, no GPU).
//! Run: cargo run --example killchain_screens -- <output-directory> [WIDTHxHEIGHT]
use red_engine2::ui::killchain;

fn main() -> anyhow::Result<()> {
    let out = std::path::PathBuf::from(std::env::args().nth(1).unwrap_or_else(|| "out/screens".into()));
    let size = std::env::args().nth(2).unwrap_or_else(|| "1280x720".into());
    let (w, h) = size.split_once('x').and_then(|(w, h)| Some((w.parse::<u32>().ok()?, h.parse::<u32>().ok()?))).unwrap_or((1280, 720));
    std::fs::create_dir_all(&out)?;
    for name in killchain::all() {
        let layout = killchain::build(name, w, h).ok_or_else(|| anyhow::anyhow!("no screen {name}"))?;
        layout.paint().to_image_over([86, 104, 118], [150, 150, 136]).save(out.join(format!("{name}.png")))?;
    }
    println!("wrote {} screens to {}", killchain::all().len(), out.display());
    Ok(())
}
