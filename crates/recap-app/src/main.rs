//! RunReCap: a menu-bar app that turns your past runs into your wallpaper.
//!
//! `runrecap` with no arguments starts the menu-bar app. Subcommands give a CLI
//! for importing, listing and rendering without the UI.

mod app;
mod background;
mod config;
mod engine;
mod update;
mod window;

use std::path::PathBuf;

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use recap_core::{gpx, Activity};
use recap_render::{Canvas, Fonts, Layout, Scene};

use crate::config::{data_dir, Config};
use crate::engine::Engine;

#[derive(Parser)]
#[command(
    name = "runrecap",
    version,
    about = "Your runs, as your Mac's wallpaper."
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Import GPX / GPX.GZ files, folders, or a Strava export (.zip or folder).
    Import { paths: Vec<PathBuf> },
    /// List runs in the library (newest first).
    List,
    /// Check for a newer version and install it (replaces RunReCap.app).
    Update {
        /// Only report whether an update is available.
        #[arg(long)]
        check: bool,
        /// Don't reopen the app after installing.
        #[arg(long)]
        no_relaunch: bool,
    },
    /// Set (or clear) the photo the route is drawn on.
    Background {
        /// Image file (JPEG, PNG, HEIC…). Omit with --clear.
        image: Option<PathBuf>,
        /// Darken the photo, 0.0–0.7.
        #[arg(long)]
        dim: Option<f64>,
        #[arg(long)]
        clear: bool,
    },
    /// Show connected displays as RunReCap sees them (size, safe-area insets).
    Screens,
    /// List available themes.
    Themes,
    /// Render a wallpaper PNG without changing the desktop.
    Render {
        /// Render this GPX file instead of the library's current run.
        #[arg(long)]
        gpx: Option<PathBuf>,
        #[arg(short, long, default_value = "runrecap.png")]
        output: PathBuf,
        #[arg(long, default_value_t = 3024)]
        width: u32,
        #[arg(long, default_value_t = 1964)]
        height: u32,
        #[arg(long, value_parser = parse_layout)]
        layout: Option<Layout>,
        /// Theme name (see `runrecap themes`), or "Automatic".
        #[arg(long)]
        theme: Option<String>,
        /// For the Automatic theme: render the light variant.
        #[arg(long)]
        light: bool,
    },
}

fn parse_layout(s: &str) -> Result<Layout, String> {
    match s.to_ascii_lowercase().as_str() {
        "single" => Ok(Layout::Single),
        "grid" => Ok(Layout::Grid),
        "heatmap" => Ok(Layout::Heatmap),
        _ => Err("expected single, grid or heatmap".into()),
    }
}

fn main() -> Result<()> {
    // Must be read before any threads start (the time crate refuses otherwise).
    let utc_offset = time::UtcOffset::current_local_offset().map_or(0, |o| o.whole_seconds());

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "runrecap=info,recap_core=info".into()),
        )
        .with_target(false)
        .init();

    let cli = Cli::parse();
    let dir = data_dir();
    match cli.command {
        None => app::run(&dir),
        Some(Command::Import { paths }) => {
            if paths.is_empty() {
                bail!("give at least one file or folder to import");
            }
            let mut engine = Engine::open(&dir)?;
            let report = engine.import(&paths);
            engine.state.save(&dir)?;
            println!("{}", report.summary());
            for (src, err) in &report.failed {
                eprintln!("  failed: {src}: {err}");
            }
            Ok(())
        }
        Some(Command::List) => {
            let engine = Engine::open(&dir)?;
            for a in engine.library.activities() {
                let date = a
                    .start
                    .map(|t| recap_render::caption::format_date(t, utc_offset))
                    .unwrap_or_default();
                println!(
                    "{}  {:<6} {:>8}  {:<28} {}",
                    a.id,
                    format!("{:?}", a.kind).to_lowercase(),
                    recap_render::caption::distance(a.distance_m, engine.config.units),
                    a.name,
                    date
                );
            }
            println!(
                "{} activities in {}",
                engine.library.activities().len(),
                engine.library.dir().display()
            );
            Ok(())
        }
        Some(Command::Update { check, no_relaunch }) => {
            let cfg = Config::load(&dir)?;
            let feed = cfg.update_feed().context(
                "no update feed configured (set update_url in config.toml or build with RUNRECAP_UPDATE_URL)",
            )?;
            println!("RunReCap {}", update::VERSION);
            let Some(release) = update::check(&feed)? else {
                println!("You're up to date.");
                return Ok(());
            };
            println!("Version {} is available.", release.version);
            if check {
                return Ok(());
            }
            let app = recap_macos::app_bundle_path().context(
                "run this from inside RunReCap.app (…/RunReCap.app/Contents/MacOS/runrecap update)",
            )?;
            println!("Downloading and verifying…");
            update::prepare(&release, &app)?.launch_swap(!no_relaunch)?;
            println!(
                "Installing {} into {} after this command exits.",
                release.version,
                app.display()
            );
            Ok(())
        }
        Some(Command::Background { image, dim, clear }) => {
            let mut cfg = Config::load(&dir)?;
            let rec = objc2::MainThreadMarker::new().map_or((2880, 1800), |mtm| {
                background::recommended_size(&recap_macos::screens(mtm))
            });
            if clear {
                cfg.background_image = None;
                println!("Background photo removed.");
            } else if let Some(path) = image {
                let dim = (dim.unwrap_or(background::DEFAULT_DIM).clamp(0.0, 0.7) * 100.0).round()
                    / 100.0;
                let bg = background::import(&dir, &path, rec, dim)?;
                println!(
                    "Background set: {} ({} × {} px, dim {:.0}%)",
                    bg.name,
                    bg.width,
                    bg.height,
                    bg.dim * 100.0
                );
                if let Some(w) = background::low_res_warning(&bg, rec) {
                    println!("warning: {w}");
                }
                cfg.background_image = Some(bg);
            } else if let (Some(d), Some(bg)) = (dim, cfg.background_image.as_mut()) {
                bg.dim = (d.clamp(0.0, 0.7) * 100.0).round() / 100.0;
                println!("Dim set to {:.0}%", bg.dim * 100.0);
            } else {
                println!(
                    "{} (JPEG, PNG, HEIC; cropped to fill the screen)",
                    background::recommendation_text(rec)
                );
                return Ok(());
            }
            cfg.save(&dir)?;
            Ok(())
        }
        Some(Command::Screens) => {
            let mtm = objc2::MainThreadMarker::new().context("must run on the main thread")?;
            for s in recap_macos::screens(mtm) {
                let [t, r, b, l] = s.safe_px;
                println!(
                    "screen {}: {}x{} px @{}x, keep-clear px top {t} right {r} bottom {b} left {l}",
                    s.id, s.width_px, s.height_px, s.scale
                );
            }
            println!(
                "appearance: {}",
                if recap_macos::system_is_dark() {
                    "dark"
                } else {
                    "light"
                }
            );
            Ok(())
        }
        Some(Command::Themes) => {
            let cfg = Config::load(&dir)?;
            println!("{} (follows system appearance)", config::AUTOMATIC);
            for t in cfg.all_themes() {
                println!("{}", t.name);
            }
            Ok(())
        }
        Some(Command::Render {
            gpx,
            output,
            width,
            height,
            layout,
            theme,
            light,
        }) => {
            let mut cfg = Config::load(&dir)?;
            if let Some(t) = theme {
                cfg.theme = t;
            }
            if let Some(l) = layout {
                cfg.layout = l;
            }
            let canvas = Canvas::new(width, height);
            let (pixmap, notes) = match gpx {
                Some(path) => render_file(&path, &cfg, canvas, !light, utc_offset, &dir)?,
                None => {
                    let mut engine = Engine::open(&dir)?;
                    engine.config = cfg;
                    let job = engine
                        .render_job(!light, utc_offset)
                        .context("library is empty; import runs first or pass --gpx FILE")?;
                    job.render(canvas, &mut Fonts::new(), true)?
                }
            };
            for w in notes.warnings {
                eprintln!("warning: {w}");
            }
            pixmap
                .save_png(&output)
                .with_context(|| format!("writing {}", output.display()))?;
            println!("wrote {}", output.display());
            Ok(())
        }
    }
}

fn render_file(
    path: &PathBuf,
    cfg: &Config,
    canvas: Canvas,
    dark: bool,
    utc_offset_s: i32,
    dir: &std::path::Path,
) -> Result<(recap_render::Pixmap, recap_render::RenderNotes)> {
    let bytes = std::fs::read(path).with_context(|| format!("reading {}", path.display()))?;
    let parsed = gpx::parse(&bytes)?;
    let kind = parsed
        .kind
        .as_deref()
        .and_then(recap_core::ActivityKind::from_label);
    let name = parsed.name.unwrap_or_else(|| "Run".into());
    let activity = Activity::from_points("cli".into(), name, kind, &parsed.points)?;
    let all = [&activity];
    let scene = Scene {
        layout: cfg.layout,
        current: &activity,
        all: &all,
        privacy_trim_m: cfg.privacy_trim_m,
        utc_offset_s,
        show_caption: true,
    };
    let theme = cfg.active_theme(dark);
    Ok(recap_render::render(
        &scene,
        &theme,
        canvas,
        &mut Fonts::new(),
        dir,
    )?)
}
