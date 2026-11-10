//! Round trip through KiCad: build each example and prove it with
//! kicad-cli (netlist, ERC, text placement). Skipped without kicad-cli.

use std::path::PathBuf;
use std::process::Command;

fn have_kicad() -> bool {
    Command::new("kicad-cli").arg("--version").output().is_ok_and(|o| o.status.success())
}

fn round_trip(input: &str, name: &str) {
    if !have_kicad() {
        eprintln!("kicad-cli not found; skipping {input}");
        return;
    }
    let out = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = std::fs::remove_dir_all(&out);
    let p = tsch::project::load(&[PathBuf::from(input)]).unwrap();
    assert!(!p.has_errors(), "{}", p.report(20));
    p.write(&out).unwrap();
    assert!(tsch::verify::verify(&p, &out).unwrap(), "verify failed for {input}");
}

#[test]
fn ldo_round_trip() {
    round_trip("examples/ldo.tsch", "ldo");
}

#[test]
fn tour_round_trip() {
    round_trip("examples/tour.tsch", "tour");
}

#[test]
fn units_round_trip() {
    round_trip("examples/opamp.tsch", "opamp");
}

#[test]
fn hierarchy_round_trip() {
    round_trip("examples/usb/project.toml", "usb");
}

#[test]
fn rebuild_is_byte_identical() {
    let p = tsch::project::load(&[PathBuf::from("examples/tour.tsch")]).unwrap();
    let a = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("det-a");
    let b = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join("det-b");
    let _ = std::fs::remove_dir_all(&a);
    let _ = std::fs::remove_dir_all(&b);
    p.write(&a).unwrap();
    p.write(&b).unwrap();
    for f in ["tour.kicad_sch", "tsch.kicad_sym"] {
        assert_eq!(std::fs::read(a.join(f)).unwrap(), std::fs::read(b.join(f)).unwrap(), "{f} differs between builds");
    }
}
