//! crosslay-editor: designs crosshairs and bakes them for the overlay.
//!
//! It's a separate program from the overlay on purpose. You only run it while
//! changing something, and closing it gives all of its memory back. Its only
//! link to the overlay is the data folder: it writes `config.toml` (for people)
//! and `overlay.bin` (pixels for the overlay), and the overlay notices.

// Release builds on Windows are GUI-subsystem apps: no console window pops up.
#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

mod canvas;
mod color;
mod config;
mod crosshair;
mod hotkey;
mod image;
mod menu;
mod png;
#[cfg(windows)]
mod win;

use config::Config;
use crosslay_format::CONFIG_FILE;
use std::path::PathBuf;

const USAGE: &str = "\
crosslay-editor - design crosshairs for the crosslay overlay

USAGE:
    crosslay-editor [OPTIONS]

With no options, opens the editor window.

OPTIONS:
    -d, --dir <FOLDER>      Use this data folder instead of the default
        --bake              Bake config.toml into overlay.bin and exit
                            (run this after editing config.toml by hand)
        --preview <FILE>    Render the startup crosshair to a PNG and exit
        --zoom <N>          Scale factor for --preview (default 8)
        --profile <N>       Crosshair index for --preview
        --print-config      Print the default config and exit
        --data-dir          Print the data folder that would be used and exit
    -h, --help              Show this help
";

enum Mode {
    Gui,
    Bake,
    Preview(PathBuf),
}

struct Args {
    dir: Option<PathBuf>,
    mode: Mode,
    zoom: u32,
    profile: Option<usize>,
}

enum Command {
    Run(Args),
    Print(String),
}

fn parse_args() -> Result<Command, String> {
    let mut args = Args {
        dir: None,
        mode: Mode::Gui,
        zoom: 8,
        profile: None,
    };
    let mut print_dir = false;
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        let arg = arg.to_string_lossy().into_owned();
        let mut value = |name: &str| {
            it.next()
                .ok_or_else(|| format!("{name} needs a value\n\n{USAGE}"))
        };
        match arg.as_str() {
            "-d" | "--dir" => args.dir = Some(value(&arg)?.into()),
            "--bake" => args.mode = Mode::Bake,
            "--preview" => args.mode = Mode::Preview(value(&arg)?.into()),
            "--zoom" => {
                args.zoom = value(&arg)?
                    .to_string_lossy()
                    .parse()
                    .map_err(|_| "--zoom expects a positive integer".to_string())?
            }
            "--profile" => {
                args.profile = Some(
                    value(&arg)?
                        .to_string_lossy()
                        .parse()
                        .map_err(|_| "--profile expects a crosshair index".to_string())?,
                )
            }
            "--print-config" => return Ok(Command::Print(config::default_config_text())),
            "--data-dir" => print_dir = true,
            "-h" | "--help" => return Ok(Command::Print(USAGE.into())),
            other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
        }
    }
    if print_dir {
        let dir = args.dir.unwrap_or_else(crosslay_format::data_dir);
        return Ok(Command::Print(dir.display().to_string()));
    }
    Ok(Command::Run(args))
}

fn run() -> Result<(), String> {
    let args = match parse_args()? {
        Command::Print(text) => {
            println!("{}", text.trim_end());
            return Ok(());
        }
        Command::Run(args) => args,
    };
    let dir = args.dir.unwrap_or_else(crosslay_format::data_dir);
    let path = dir.join(CONFIG_FILE);

    match args.mode {
        Mode::Gui => menu::run(dir),
        Mode::Bake => {
            let cfg = Config::load_or_create(&path)?;
            cfg.save(&dir)?;
            println!(
                "baked {} crosshair(s) into {}",
                cfg.crosshairs.len(),
                dir.join(crosslay_format::BAKED_FILE).display()
            );
            Ok(())
        }
        Mode::Preview(out) => {
            let mut cfg = Config::load_or_create(&path)?;
            if let Some(p) = args.profile {
                cfg.profile = p;
            }
            let ch = &cfg.crosshairs[cfg.profile % cfg.crosshairs.len()];
            let canvas = ch.render(&dir)?;
            let png = png::encode(&canvas, args.zoom);
            std::fs::write(&out, png)
                .map_err(|e| format!("cannot write {}: {e}", out.display()))?;
            println!("wrote {} ({})", out.display(), ch.name);
            Ok(())
        }
    }
}

fn main() {
    #[cfg(windows)]
    if std::env::args_os().len() > 1 {
        win::attach_console();
    }
    if let Err(msg) = run() {
        #[cfg(windows)]
        win::show_error(&msg);
        eprintln!("crosslay-editor: {msg}");
        std::process::exit(1);
    }
}
