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
mod windows;

use std::path::PathBuf;

const USAGE: &str = "\
crosslay - a zero-overhead crosshair overlay

USAGE:
    crosslay [OPTIONS]

OPTIONS:
    -c, --config <FILE>      Use this config file instead of the default location
    -m, --menu               Open the crosshair editor instead of the overlay
        --preview <FILE>     Render the startup crosshair to a PNG and exit
        --zoom <N>           Scale factor for --preview (default 8)
        --profile <N>        Override the startup profile index
        --print-config       Print the default config and exit
        --config-path        Print the config path that would be used and exit
    -h, --help               Show this help
";

struct Args {
    config: Option<PathBuf>,
    preview: Option<PathBuf>,
    menu: bool,
    zoom: u32,
    profile: Option<usize>,
}

enum Command {
    Run(Args),
    Exit(String),
}

fn parse_args() -> Result<Command, String> {
    let mut args = Args {
        config: None,
        preview: None,
        menu: false,
        zoom: 8,
        profile: None,
    };
    let mut it = std::env::args_os().skip(1);
    while let Some(arg) = it.next() {
        let arg = arg.to_string_lossy().into_owned();
        let mut value = |name: &str| {
            it.next()
                .ok_or_else(|| format!("{name} needs a value\n\n{USAGE}"))
        };
        match arg.as_str() {
            "-c" | "--config" => args.config = Some(value(&arg)?.into()),
            "-m" | "--menu" => args.menu = true,
            "--preview" => args.preview = Some(value(&arg)?.into()),
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
                        .map_err(|_| "--profile expects a profile index".to_string())?,
                )
            }
            "--print-config" => return Ok(Command::Exit(config::default_config_text())),
            "--config-path" => {
                return Ok(Command::Exit(config::default_path().display().to_string()));
            }
            "-h" | "--help" => return Ok(Command::Exit(USAGE.into())),
            other => return Err(format!("unknown argument {other:?}\n\n{USAGE}")),
        }
    }
    Ok(Command::Run(args))
}

fn run() -> Result<(), String> {
    let args = match parse_args()? {
        Command::Exit(text) => {
            println!("{}", text.trim_end());
            return Ok(());
        }
        Command::Run(args) => args,
    };

    let path = args.config.unwrap_or_else(config::default_path);
    if args.menu {
        return menu::run(path);
    }
    let mut cfg = config::Config::load_or_create(&path)?;
    if let Some(p) = args.profile {
        cfg.profile = p;
    }

    if let Some(out) = args.preview {
        let canvas = cfg.active().render(&config::base_dir(&path))?;
        let png = png::encode(&canvas, args.zoom);
        std::fs::write(&out, png).map_err(|e| format!("cannot write {}: {e}", out.display()))?;
        println!("wrote {} ({})", out.display(), cfg.active().name);
        return Ok(());
    }

    #[cfg(windows)]
    {
        windows::run(path, cfg)
    }
    #[cfg(not(windows))]
    {
        let _ = cfg;
        Err("the live overlay currently runs on Windows only; \
             use --menu to edit crosshairs or --preview <FILE> to render one"
            .into())
    }
}

fn main() {
    #[cfg(windows)]
    if std::env::args_os().len() > 1 {
        windows::attach_console();
    }
    if let Err(msg) = run() {
        #[cfg(windows)]
        windows::show_error(&msg);
        eprintln!("crosslay: {msg}");
        std::process::exit(1);
    }
}
