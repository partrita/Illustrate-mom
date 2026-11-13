use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};

use illustrate::pnm::{parse_ppm_p3, write_ppm_p3};
use illustrate::run_command_script;

#[test]
fn parity_against_legacy_fortran_2hhb() {
    let manifest_dir = Path::new(env!("CARGO_MANIFEST_DIR"));
    let inp_path = manifest_dir.join("examples/2hhb.inp");
    let pdb_path = manifest_dir.join("examples/2hhb.pdb");
    let fortran_src = manifest_dir.join("legacy/illustrate.f");

    let script = fs::read_to_string(&inp_path).expect("read examples/2hhb.inp");
    let (_deck, pdb, rust_res) = run_command_script(&script, Some(&manifest_dir.join("examples")))
        .expect("Rust render of 2hhb");

    assert_eq!(pdb.atom_count(), 4556);
    assert_eq!(pdb.num_subunits, 8);
    assert_eq!(pdb.bio_chains, vec![b'A', b'B', b'C', b'D']);
    assert_eq!(pdb.biomats.len(), 1);
    assert_eq!(rust_res.width, 922);
    assert_eq!(rust_res.height, 750);

    // Check if gfortran is available; if so, run reference binary and verify 0 pixel differences.
    if Command::new("gfortran").arg("--version").output().is_err() {
        return;
    }

    let tmp_dir = std::env::temp_dir().join(format!("illustrate_parity_{}", std::process::id()));
    fs::create_dir_all(&tmp_dir).expect("create temp dir");

    let bin_path = tmp_dir.join("illustrate_ref");
    let compile_status = Command::new("gfortran")
        .arg("-finit-local-zero")
        .arg(&fortran_src)
        .arg("-o")
        .arg(&bin_path)
        .status()
        .expect("invoke gfortran");
    assert!(compile_status.success(), "gfortran compilation failed");

    fs::copy(&pdb_path, tmp_dir.join("2hhb.pdb")).expect("copy 2hhb.pdb");
    let inp_file = fs::File::open(&inp_path).expect("open 2hhb.inp");
    let run_output = Command::new(&bin_path)
        .current_dir(&tmp_dir)
        .stdin(Stdio::from(inp_file))
        .output()
        .expect("run legacy binary");
    assert!(run_output.status.success(), "legacy binary failed");

    let ref_color_text = fs::read_to_string(tmp_dir.join("2hhb.pnm")).expect("read ref 2hhb.pnm");
    let ref_opacity_text =
        fs::read_to_string(tmp_dir.join("opacity.pnm")).expect("read ref opacity.pnm");

    let (ref_w, ref_h, ref_rgb) = parse_ppm_p3(&ref_color_text).expect("parse ref 2hhb.pnm");
    let (op_w, op_h, ref_op) = parse_ppm_p3(&ref_opacity_text).expect("parse ref opacity.pnm");

    assert_eq!((rust_res.width, rust_res.height), (ref_w, ref_h));
    assert_eq!((rust_res.width, rust_res.height), (op_w, op_h));

    let color_mismatches = rust_res
        .rgb
        .iter()
        .zip(ref_rgb.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(color_mismatches, 0, "RGB pixel mismatch count must be 0");

    let opacity_mismatches = rust_res
        .opacity
        .iter()
        .zip(ref_op.iter())
        .filter(|(a, b)| a != b)
        .count();
    assert_eq!(
        opacity_mismatches, 0,
        "Opacity pixel mismatch count must be 0"
    );

    // Also verify formatted P3 output matches byte-for-byte
    let mut rust_color_bytes = Vec::new();
    write_ppm_p3(
        &mut rust_color_bytes,
        rust_res.width,
        rust_res.height,
        &rust_res.rgb,
    )
    .unwrap();
    let rust_color_text = String::from_utf8(rust_color_bytes).unwrap();
    assert_eq!(rust_color_text, ref_color_text);

    let mut rust_opacity_bytes = Vec::new();
    write_ppm_p3(
        &mut rust_opacity_bytes,
        rust_res.width,
        rust_res.height,
        &rust_res.opacity,
    )
    .unwrap();
    let rust_opacity_text = String::from_utf8(rust_opacity_bytes).unwrap();
    assert_eq!(rust_opacity_text, ref_opacity_text);

    let _ = fs::remove_dir_all(&tmp_dir);
}
