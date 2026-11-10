use tsch::{diag, project, show, verify};

use std::path::PathBuf;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

use diag::Severity;

#[derive(Parser)]
#[command(name = "tsch", version, about = "Text schematics to KiCad: one character is one 1.27 x 2.54 mm grid cell")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Check the rules; print errors and warnings with line:column.
    Check {
        /// A .tsch sheet, several sheets, or a project .toml listing sheets.
        inputs: Vec<PathBuf>,
        /// Most findings to print per file.
        #[arg(long, default_value_t = 60)]
        max: usize,
    },
    /// Check, then write the KiCad schematic and project files.
    Build {
        inputs: Vec<PathBuf>,
        #[arg(short, long, default_value = "out")]
        out: PathBuf,
    },
    /// Build, then prove the result with kicad-cli: the netlist KiCad reads
    /// back equals the drawn one, ERC, and every rendered text inside its
    /// cells with nothing drawn over it.
    Verify {
        inputs: Vec<PathBuf>,
        #[arg(short, long, default_value = "out")]
        out: PathBuf,
    },
    /// Print the nets as the drawing defines them.
    Netlist { inputs: Vec<PathBuf> },
    /// Print part of a sheet with line numbers and a column ruler.
    Show {
        file: PathBuf,
        /// File lines, e.g. 20:60 (1-based, inclusive).
        #[arg(long)]
        lines: Option<String>,
        /// Columns, e.g. 1:120 (1-based, inclusive).
        #[arg(long)]
        cols: Option<String>,
    },
    /// Build and plot each sheet to PNG (needs inkscape).
    Render {
        inputs: Vec<PathBuf>,
        #[arg(short, long, default_value = "out")]
        out: PathBuf,
        /// Pixels per mm.
        #[arg(long, default_value_t = 8.0)]
        scale: f64,
    },
    /// Print an aligned box for an IC or connector. Pins are NUM, NUM:NAME
    /// or NUM:NAME=NET, comma separated; a net on the left or right becomes
    /// a label ([NET] global), on the top or bottom a power port; no net
    /// marks the pin unused (x).
    Box {
        #[arg(long = "ref")]
        reference: String,
        #[arg(long)]
        value: String,
        #[arg(long, default_value = "")]
        left: String,
        #[arg(long, default_value = "")]
        right: String,
        #[arg(long, default_value = "")]
        top: String,
        #[arg(long, default_value = "")]
        bottom: String,
    },
    /// Print the text schematic format specification.
    Spec,
}

fn load(inputs: &[PathBuf], max: usize) -> Result<project::Project, ExitCode> {
    if inputs.is_empty() {
        eprintln!("no input files");
        return Err(ExitCode::from(2));
    }
    match project::load(inputs) {
        Ok(p) => {
            let rep = p.report(max);
            if !rep.is_empty() {
                eprint!("{rep}");
            }
            Ok(p)
        }
        Err(e) => {
            eprintln!("error: {e}");
            Err(ExitCode::from(2))
        }
    }
}

fn summary(p: &project::Project) -> String {
    let parts: usize = p.sheets.iter().map(|s| s.sheet.parts.len()).sum();
    let nets = p.netlist().len();
    format!(
        "{} error(s), {} warning(s); {} sheet(s), {} part(s), {} net(s)",
        p.count(Severity::Error),
        p.count(Severity::Warning),
        p.sheets.len(),
        parts,
        nets
    )
}

fn build(inputs: &[PathBuf], out: &PathBuf) -> Result<project::Project, ExitCode> {
    let p = load(inputs, 60)?;
    eprintln!("{}", summary(&p));
    if p.has_errors() {
        eprintln!("not written: fix the errors first");
        return Err(ExitCode::from(1));
    }
    match p.write(out) {
        Ok(files) => {
            for f in files {
                eprintln!("wrote {}", f.display());
            }
            Ok(p)
        }
        Err(e) => {
            eprintln!("error: {e}");
            Err(ExitCode::from(1))
        }
    }
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    match cli.cmd {
        Cmd::Check { inputs, max } => match load(&inputs, max) {
            Ok(p) => {
                eprintln!("{}", summary(&p));
                if p.has_errors() { ExitCode::from(1) } else { ExitCode::SUCCESS }
            }
            Err(c) => c,
        },
        Cmd::Build { inputs, out } => match build(&inputs, &out) {
            Ok(_) => ExitCode::SUCCESS,
            Err(c) => c,
        },
        Cmd::Verify { inputs, out } => match build(&inputs, &out) {
            Ok(p) => match verify::verify(&p, &out) {
                Ok(true) => ExitCode::SUCCESS,
                Ok(false) => ExitCode::from(1),
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::from(2)
                }
            },
            Err(c) => c,
        },
        Cmd::Netlist { inputs } => match load(&inputs, 60) {
            Ok(p) => {
                for n in p.netlist() {
                    let name = if n.names.is_empty() { "(unnamed)".to_string() } else { n.names.join(" = ") };
                    let pins: Vec<String> = n.pins.iter().map(|(r, p)| format!("{r}.{p}")).collect();
                    println!("{name}: {}", pins.join(" "));
                }
                if p.has_errors() { ExitCode::from(1) } else { ExitCode::SUCCESS }
            }
            Err(c) => c,
        },
        Cmd::Show { file, lines, cols } => match show::show(&file, lines.as_deref(), cols.as_deref()) {
            Ok(s) => {
                print!("{s}");
                ExitCode::SUCCESS
            }
            Err(e) => {
                eprintln!("error: {e}");
                ExitCode::from(2)
            }
        },
        Cmd::Render { inputs, out, scale } => match build(&inputs, &out) {
            Ok(p) => match verify::render(&p, &out, scale) {
                Ok(files) => {
                    for f in files {
                        println!("{}", f.display());
                    }
                    ExitCode::SUCCESS
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    ExitCode::from(2)
                }
            },
            Err(c) => c,
        },
        Cmd::Box { reference, value, left, right, top, bottom } => {
            let lists = [&left, &right, &top, &bottom].map(|s| tsch::boxgen::parse_list(s));
            match lists {
                [Ok(l), Ok(r), Ok(t), Ok(b)] => {
                    print!("{}", tsch::boxgen::generate(&reference, &value, &l, &r, &t, &b));
                    ExitCode::SUCCESS
                }
                other => {
                    for e in other.into_iter().filter_map(|x| x.err()) {
                        eprintln!("error: {e}");
                    }
                    ExitCode::from(2)
                }
            }
        }
        Cmd::Spec => {
            print!("{}", include_str!("../FORMAT.md"));
            ExitCode::SUCCESS
        }
    }
}
