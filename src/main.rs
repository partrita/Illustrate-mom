use std::fs;
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::process;

use clap::Parser;

use illustrate::pnm::{save_png_file, save_ppm_file};
use illustrate::run_command_script;

/// Biomolecular non-photorealistic renderer (Rust port of David S. Goodsell's Illustrate).
#[derive(Debug, Parser)]
#[command(name = "illustrate", version, about)]
struct Cli {
    /// Path to a command card file (reads from standard input if omitted).
    #[arg(value_name = "INPUT_FILE")]
    input: Option<PathBuf>,

    /// Override the output PPM filename specified by the `calculate` card.
    #[arg(short, long, value_name = "FILE")]
    output: Option<PathBuf>,

    /// Override the opacity PPM filename (defaults to `opacity.pnm`).
    #[arg(long, value_name = "FILE")]
    opacity_output: Option<PathBuf>,

    /// Skip writing the `opacity.pnm` file.
    #[arg(long)]
    no_opacity: bool,

    /// Also export an RGBA PNG image combining RGB color and opacity.
    #[arg(long, value_name = "FILE")]
    png: Option<PathBuf>,

    /// Suppress diagnostic stdout output.
    #[arg(short, long)]
    quiet: bool,
}

fn run() -> illustrate::Result<()> {
    let cli = Cli::parse();

    let (script, script_dir) = match &cli.input {
        Some(path) => {
            let content = fs::read_to_string(path)?;
            let parent = path.parent().filter(|p| !p.as_os_str().is_empty());
            (content, parent.map(Path::to_path_buf))
        }
        None => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            (buf, None)
        }
    };

    let (deck, pdb, result) = run_command_script(&script, script_dir.as_deref())?;

    let color_out = cli
        .output
        .or_else(|| deck.output_path.clone())
        .unwrap_or_else(|| PathBuf::from("illustrate.pnm"));

    save_ppm_file(&color_out, result.width, result.height, &result.rgb)?;

    if !cli.no_opacity {
        let opacity_out = cli
            .opacity_output
            .unwrap_or_else(|| PathBuf::from("opacity.pnm"));
        save_ppm_file(&opacity_out, result.width, result.height, &result.opacity)?;
    }

    if let Some(png_out) = &cli.png {
        save_png_file(png_out, &result)?;
    }

    if !cli.quiet {
        println!("atom descriptors: {}", deck.descriptors.len());
        println!(
            "atoms read: {} from {}",
            pdb.atom_count(),
            deck.pdb_path.display()
        );
        println!("number of subunits: {}", pdb.num_subunits);
        if !pdb.bio_chains.is_empty() {
            let chains: Vec<String> = pdb
                .bio_chains
                .iter()
                .map(|&b| (b as char).to_string())
                .collect();
            println!("chains in biological assembly: {}", chains.join(" "));
        }
        println!("number of BIOMT: {}", pdb.biomats.len());
        println!(
            "image size (width x height): {} x {}",
            result.width, result.height
        );
        println!(
            "zpix_min, zpix_max: {} {}",
            result.z_range.0, result.z_range.1
        );
        for (idx, count) in result.spheres_per_type.iter().enumerate() {
            println!("{count} spheres added of type {}", idx + 1);
        }
        println!("output pnm filename: {}", color_out.display());
    }

    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("error: {err}");
        process::exit(1);
    }
}
