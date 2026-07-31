use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

// A monotonic per-process counter makes temp paths unique even when two tests
// run in parallel and read the same nanosecond clock value (the timestamp alone
// is not collision-proof under the libtest thread pool).
static TEMP_SEQ: AtomicU64 = AtomicU64::new(0);

fn temp_path(ext: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock before unix epoch")
        .as_nanos();
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    std::env::temp_dir().join(format!(
        "strok-cli-{}-{}-{}.{}",
        std::process::id(),
        nanos,
        seq,
        ext
    ))
}

fn write_temp_strok(contents: &str) -> PathBuf {
    let path = temp_path("strok");
    fs::write(&path, contents).expect("write temp .strok");
    path
}

fn cleanup(path: &Path) {
    let _ = fs::remove_file(path);
}

fn png_dimensions(bytes: &[u8]) -> (u32, u32) {
    assert!(bytes.len() >= 24, "PNG is too short");
    (
        u32::from_be_bytes(bytes[16..20].try_into().unwrap()),
        u32::from_be_bytes(bytes[20..24].try_into().unwrap()),
    )
}

#[test]
fn inspect_svg_accepts_shape_name() {
    let strok_path = write_temp_strok(concat!(
        "documentsize 120x120\n",
        "\n",
        "shape badge template=path\n",
        "  addpoint start at=0,0\n",
        "  addpoint end at=100,100 mode=controls c1=25,0 c2=100,75\n",
        "\n",
        "place placed shape=badge at=10,10\n",
    ));

    let output = Command::new(env!("CARGO_BIN_EXE_strok"))
        .args([
            "-f",
            strok_path.to_str().unwrap(),
            "inspect",
            "badge",
            "--svg",
        ])
        .output()
        .expect("run strok inspect");

    cleanup(&strok_path);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(stdout.contains("id=\"__preview_badge__\""));
    assert!(stdout.contains("C25 0, 100 75, 100 100"));
}

#[test]
fn inspect_structural_keeps_live_boolean_operands_named() {
    let strok_path = write_temp_strok(concat!(
        "documentsize 100x100\n",
        "\n",
        "shape block template=rectangle\n",
        "\n",
        "boolean silhouette op=union\n",
        "  place head shape=block at=10,10 size=30x30\n",
        "  place neck shape=block at=25,25 size=20x45 rotation=8\n",
        "  fill #f7f3ea\n",
    ));

    let output = Command::new(env!("CARGO_BIN_EXE_strok"))
        .args([
            "-f",
            strok_path.to_str().unwrap(),
            "inspect",
            "--detail",
            "structural",
        ])
        .output()
        .expect("run strok inspect");

    cleanup(&strok_path);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).expect("stdout utf8");
    assert!(stdout.contains("silhouette (boolean-union)"), "{stdout}");
    assert!(stdout.contains("head (rectangle)"), "{stdout}");
    assert!(stdout.contains("neck (rectangle)"), "{stdout}");
}

#[test]
fn render_node_accepts_shape_name() {
    let strok_path = write_temp_strok(concat!(
        "documentsize 120x120\n",
        "\n",
        "shape badge template=rectangle\n",
        "\n",
        "place placed shape=badge at=10,10 size=80x50\n",
    ));
    let png_path = temp_path("png");

    let output = Command::new(env!("CARGO_BIN_EXE_strok"))
        .args([
            "-f",
            strok_path.to_str().unwrap(),
            "render",
            "--node",
            "badge",
            "--out",
            png_path.to_str().unwrap(),
        ])
        .output()
        .expect("run strok render");

    let png = fs::read(&png_path).expect("read rendered png");
    cleanup(&strok_path);
    cleanup(&png_path);

    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(png.len() > 8);
    assert_eq!(&png[0..4], &[0x89, 0x50, 0x4e, 0x47]);
}

#[test]
fn new_icon_profile_seeds_defaults() {
    let strok_path = temp_path("strok");
    let output = Command::new(env!("CARGO_BIN_EXE_strok"))
        .args(["new", strok_path.to_str().unwrap(), "--profile", "icon"])
        .output()
        .expect("run strok new --profile icon");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("round-outline compatibility alias"),
        "legacy profile should teach the explicit style choice"
    );

    let contents = fs::read_to_string(&strok_path).expect("read seeded icon");
    cleanup(&strok_path);

    // 24x24 grid + a stroke defaults block tuned for outline icons.
    assert!(contents.contains("documentsize 24x24"));
    assert!(contents.contains("defaults"));
    assert!(contents.contains("stroke currentColor"));
    assert!(contents.contains("stroke-width 2"));
    assert!(contents.contains("stroke-linecap round"));
    assert!(contents.contains("fill none"));
}

#[test]
fn new_icon_profile_honors_explicit_size() {
    let strok_path = temp_path("strok");
    let output = Command::new(env!("CARGO_BIN_EXE_strok"))
        .args([
            "new",
            strok_path.to_str().unwrap(),
            "16x16",
            "--profile",
            "icon",
        ])
        .output()
        .expect("run strok new 16x16 --profile icon");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let contents = fs::read_to_string(&strok_path).expect("read seeded icon");
    cleanup(&strok_path);
    assert!(contents.contains("documentsize 16x16"));
    assert!(contents.contains("stroke currentColor"));
}

#[test]
fn icon_profiles_make_visual_grammar_explicit() {
    let cases = [
        (
            "icon-outline-angular",
            "stroke-linecap butt",
            "stroke-linejoin miter",
        ),
        ("icon-solid", "fill currentColor", "stroke none"),
        (
            "icon-mixed",
            "mixed solid + line",
            "do not outline every filled component",
        ),
    ];
    for (profile, expected_a, expected_b) in cases {
        let strok_path = temp_path("strok");
        let output = Command::new(env!("CARGO_BIN_EXE_strok"))
            .args(["new", strok_path.to_str().unwrap(), "--profile", profile])
            .output()
            .expect("run strok new with visual profile");
        assert!(
            output.status.success(),
            "{profile}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        let contents = fs::read_to_string(&strok_path).unwrap();
        cleanup(&strok_path);
        assert!(contents.contains(expected_a), "{profile}: {contents}");
        assert!(contents.contains(expected_b), "{profile}: {contents}");
    }
}

#[test]
fn guide_teaches_visual_decisions_and_review_loop() {
    for topic in ["illustration", "icon", "logo", "diagram"] {
        let output = Command::new(env!("CARGO_BIN_EXE_strok"))
            .args(["guide", topic])
            .output()
            .expect("run strok guide");
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let stdout = String::from_utf8(output.stdout).unwrap();
        assert!(stdout.contains("WORKFLOW"), "{stdout}");
        assert!(
            stdout.contains("size") || stdout.contains("baseline"),
            "{stdout}"
        );
        assert!(
            stdout.contains("iterate") || stdout.contains("Iterate"),
            "{stdout}"
        );
    }
}

#[test]
fn agent_intro_sets_effort_and_requires_focused_visual_review() {
    let output = Command::new(env!("CARGO_BIN_EXE_strok"))
        .arg("agent-intro")
        .output()
        .expect("run strok agent-intro");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stdout = String::from_utf8(output.stdout).unwrap();
    for expected in [
        "CHOOSE THE EFFORT LEVEL",
        "sketch",
        "production",
        "showcase",
        "render --region",
        "render --outline",
        "live `boolean",
        "thumbnail or",
        "Technically valid",
    ] {
        assert!(stdout.contains(expected), "missing '{expected}':\n{stdout}");
    }
}

#[test]
fn root_help_directs_agents_to_the_intro() {
    let output = Command::new(env!("CARGO_BIN_EXE_strok"))
        .arg("--help")
        .output()
        .expect("run strok --help");

    assert!(output.status.success());
    let stdout = String::from_utf8(output.stdout).unwrap();
    assert!(
        stdout.contains("AGENTS: Run `strok agent-intro`"),
        "{stdout}"
    );
}

#[test]
fn render_region_outputs_a_high_resolution_document_crop() {
    let strok_path = write_temp_strok(concat!(
        "documentsize 200x100\n",
        "shape left template=rectangle\n",
        "  fill #ff0000\n",
        "shape right template=rectangle\n",
        "  fill #0000ff\n",
        "place left shape=left at=0,0 size=100x100\n",
        "place right shape=right at=100,0 size=100x100\n",
    ));
    let png_path = temp_path("png");
    let output = Command::new(env!("CARGO_BIN_EXE_strok"))
        .args([
            "-f",
            strok_path.to_str().unwrap(),
            "render",
            "--region",
            "100,0,100,100",
            "--width",
            "600",
            "--out",
            png_path.to_str().unwrap(),
        ])
        .output()
        .expect("run strok render --region");

    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    let png = fs::read(&png_path).unwrap();
    cleanup(&strok_path);
    cleanup(&png_path);
    assert_eq!(png_dimensions(&png), (600, 600));
}

#[test]
fn batch_renders_svg_and_png_for_each_file() {
    // A scratch icon directory with two files.
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let dir = std::env::temp_dir().join(format!("strok-batch-{}-{}", std::process::id(), nanos));
    fs::create_dir_all(&dir).unwrap();

    let icon = "\
documentsize 24x24

defaults
  fill none
  stroke currentColor
  stroke-width 2
  stroke-linecap round

shape h template=line
  stroke currentColor

place h shape=h at=4,12 size=16x0
";
    fs::write(dir.join("plus.strok"), icon).unwrap();
    fs::write(dir.join("minus.strok"), icon).unwrap();

    let out = dir.join("dist");
    let output = Command::new(env!("CARGO_BIN_EXE_strok"))
        .args([
            "batch",
            dir.to_str().unwrap(),
            "--out",
            out.to_str().unwrap(),
            "--sizes",
            "16,24",
            "--color",
            "#1a1a1a",
        ])
        .output()
        .expect("run strok batch");
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // One SVG per file (currentColor preserved) + one PNG per file per size.
    let plus_svg = fs::read_to_string(out.join("plus.svg")).expect("plus.svg");
    assert!(plus_svg.contains("currentColor"));
    assert!(out.join("plus-16.png").exists());
    assert!(out.join("plus-24.png").exists());
    assert!(out.join("minus-16.png").exists());
    assert!(out.join("minus-24.png").exists());

    let _ = fs::remove_dir_all(&dir);
}

// ── C3: boolean ops / outline-stroke / offset ───────────────────────────

fn run(args: &[&str]) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_strok"))
        .args(args)
        .output()
        .expect("run strok")
}

#[test]
fn bool_subtract_writes_path_shape_and_renders() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape body template=rectangle\n  fill #3b82f6\n",
        "shape tab template=rectangle\n  fill #ff0000\n",
        "place body shape=body at=15,25 size=70x55\n",
        "place tab shape=tab at=25,15 size=28x18\n",
    ));
    let f = p.to_str().unwrap();
    let o = run(&[
        "-f", f, "bool", "subtract", "body", "tab", "--out", "folder",
    ]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );

    // The new shape + place landed and resolve to SVG.
    let svg = run(&["-f", f, "inspect", "folder", "--svg"]);
    cleanup(&p);
    assert!(svg.status.success());
    let s = String::from_utf8(svg.stdout).unwrap();
    assert!(s.contains("id=\"folder\""), "result place missing: {s}");
    assert!(s.contains("<path"), "result should be a path");
}

#[test]
fn bool_subtract_hole_uses_even_odd() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape outer template=rectangle\n  fill #c8863a\n",
        "shape inner template=rectangle\n  fill #ff0000\n",
        "place outer shape=outer at=20,20 size=60x60\n",
        "place inner shape=inner at=40,40 size=20x20\n",
    ));
    let f = p.to_str().unwrap();
    assert!(
        run(&["-f", f, "bool", "subtract", "outer", "inner", "--out", "donut"])
            .status
            .success()
    );
    let svg = run(&["-f", f, "inspect", "donut", "--svg"]);
    cleanup(&p);
    let s = String::from_utf8(svg.stdout).unwrap();
    assert!(
        s.contains("fill-rule=\"evenodd\""),
        "hole needs even-odd: {s}"
    );
    // outer + hole = two subpaths (two `M` move-tos in the one path `d`).
    let d_line = s
        .lines()
        .find(|l| l.contains("id=\"donut\""))
        .expect("donut path line");
    assert!(
        d_line.matches('M').count() >= 2,
        "expected 2 subpaths: {d_line}"
    );
}

#[test]
fn outline_stroke_produces_filled_path() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape wire template=path\n",
        "  stroke #000000\n  stroke-width 10\n  stroke-linecap round\n",
        "  addpoint a at=10,50\n  addpoint b at=90,50\n",
        "place wire shape=wire at=0,0\n",
    ));
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "outline-stroke", "wire", "--out", "filled"]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let svg = run(&["-f", f, "inspect", "filled", "--svg"]);
    cleanup(&p);
    let s = String::from_utf8(svg.stdout).unwrap();
    assert!(s.contains("id=\"filled\""));
}

#[test]
fn offset_grows_and_round_trips() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape disc template=ellipse\n  fill #000000\n",
        "place disc shape=disc at=30,30 size=40x40\n",
    ));
    let f = p.to_str().unwrap();
    assert!(run(&["-f", f, "offset", "disc", "5", "--out", "grown"])
        .status
        .success());
    // emit is idempotent (round-trip stable)
    let a = run(&["-f", f, "inspect"]);
    let txt = String::from_utf8(a.stdout).unwrap();
    let p2 = write_temp_strok(&txt);
    let b = run(&["-f", p2.to_str().unwrap(), "inspect"]);
    cleanup(&p);
    cleanup(&p2);
    assert_eq!(
        txt,
        String::from_utf8(b.stdout).unwrap(),
        "round-trip unstable"
    );
}

#[test]
fn bool_unknown_op_errors_cleanly() {
    let p = write_temp_strok("documentsize 50x50\n\nshape a template=rectangle\n  fill #000000\nplace a shape=a at=0,0 size=50x50\n");
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "bool", "frobnicate", "a", "a", "--out", "x"]);
    cleanup(&p);
    assert!(!o.status.success());
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("unknown boolean op"), "stderr: {e}");
    assert!(!e.contains("panicked"), "must not panic: {e}");
}

#[test]
fn bool_missing_input_errors_cleanly() {
    let p = write_temp_strok("documentsize 50x50\n\nshape a template=rectangle\n  fill #000000\nplace a shape=a at=0,0 size=50x50\n");
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "bool", "union", "a", "ghost", "--out", "x"]);
    cleanup(&p);
    assert!(!o.status.success());
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("ghost"), "should name missing input: {e}");
}

// ── C4: transform / convert-point CLI verbs (E2.3, E2.5) ─────────────────

#[test]
fn transform_rotate_accumulates_and_round_trips() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape card template=rectangle\n  fill #4488cc\n",
        "place c shape=card at=20,20 size=40x40\n",
    ));
    let f = p.to_str().unwrap();
    assert!(run(&["-f", f, "transform", "c", "--rotate", "15"])
        .status
        .success());
    // Rotate again — should accumulate to 30.
    assert!(run(&["-f", f, "transform", "c", "--rotate", "15"])
        .status
        .success());
    let txt = String::from_utf8(run(&["-f", f, "inspect"]).stdout).unwrap();
    cleanup(&p);
    assert!(
        txt.contains("rotation=30deg"),
        "accumulated rotation: {txt}"
    );
}

#[test]
fn transform_skew_and_flip_render() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape card template=rectangle\n  fill #4488cc\n",
        "place c shape=card at=20,20 size=40x40\n",
    ));
    let f = p.to_str().unwrap();
    assert!(run(&["-f", f, "transform", "c", "--skew", "20,0"])
        .status
        .success());
    assert!(run(&["-f", f, "transform", "c", "--flip", "x"])
        .status
        .success());
    let svg = String::from_utf8(run(&["-f", f, "inspect", "--svg"]).stdout).unwrap();
    cleanup(&p);
    // Skew bakes into the element matrix; flip into the placed `d`.
    assert!(svg.contains("transform=\"matrix("), "skew matrix: {svg}");
}

#[test]
fn transform_missing_place_errors_cleanly() {
    let p = write_temp_strok("documentsize 50x50\n\nshape a template=rectangle\n  fill #000000\nplace a shape=a at=0,0 size=50x50\n");
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "transform", "ghost", "--rotate", "10"]);
    cleanup(&p);
    assert!(!o.status.success());
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("ghost"), "should name missing place: {e}");
    assert!(!e.contains("panicked"), "must not panic: {e}");
}

#[test]
fn convert_point_changes_mode_and_round_trips() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape kite template=path\n",
        "  addpoint top at=50,10\n  addpoint right at=90,50\n",
        "  addpoint bottom at=50,90\n  addpoint left at=10,50\n  close\n",
        "  fill #c8863a\n",
        "place kite shape=kite at=0,0\n",
    ));
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "convert-point", "kite.bottom", "--to", "arc"]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let svg = String::from_utf8(run(&["-f", f, "inspect", "--svg"]).stdout).unwrap();
    cleanup(&p);
    assert!(svg.contains(" A"), "bottom should render as an arc: {svg}");
}

#[test]
fn convert_point_bad_target_errors_cleanly() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape k template=path\n  addpoint a at=10,10\n  addpoint b at=90,90\n",
        "place k shape=k at=0,0\n",
    ));
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "convert-point", "k.b", "--to", "wobbly"]);
    cleanup(&p);
    assert!(!o.status.success());
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("not a valid target"), "stderr: {e}");
    assert!(!e.contains("panicked"), "must not panic: {e}");
}

// ── C5 (E2.6 / E2.7) ─────────────────────────────────────────────────────

#[test]
fn per_corner_round_trips_and_renders_mixed() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape card template=rectangle\n",
        "  round-corners tl=16 tr=16 br=0 bl=0\n  fill #c8863a\n",
        "place card shape=card at=10,10 size=80x80\n",
    ));
    let f = p.to_str().unwrap();
    let txt = String::from_utf8(run(&["-f", f, "inspect"]).stdout).unwrap();
    let svg = String::from_utf8(run(&["-f", f, "inspect", "--svg"]).stdout).unwrap();
    cleanup(&p);
    // Per-corner spelling round-trips verbatim.
    assert!(
        txt.contains("round-corners tl=16 tr=16 br=0 bl=0"),
        "per-corner round-trip: {txt}"
    );
    // Rounded corners ⇒ arcs present; sharp corners ⇒ straight `L` corners remain.
    assert!(
        svg.contains(" A"),
        "rounded corners should emit arcs: {svg}"
    );
}

#[test]
fn notch_folder_tab_pokes_outward() {
    // A folder = rectangle + ONE outward tab on the top edge — vs the 6-point
    // hand-composed folder icon. The tab must rise ABOVE the body top.
    let p = write_temp_strok(concat!(
        "documentsize 24x24\n\n",
        "shape folder template=rectangle\n",
        "  notch edge=top dir=out shape=square pos=0.3 width=8 depth=3\n",
        "  fill none\n  stroke currentColor\n  stroke-width 2\n",
        "place folder shape=folder at=2,6 size=20x12\n",
    ));
    let f = p.to_str().unwrap();
    let txt = String::from_utf8(run(&["-f", f, "inspect"]).stdout).unwrap();
    let svg = String::from_utf8(run(&["-f", f, "inspect", "--svg"]).stdout).unwrap();
    cleanup(&p);
    assert!(
        txt.contains("notch edge=top dir=out"),
        "notch round-trip: {txt}"
    );
    assert!(svg.contains("<path"), "renders: {svg}");
}

#[test]
fn text_on_path_emits_textpath() {
    let p = write_temp_strok(concat!(
        "documentsize 200x100\n\n",
        "shape arch template=path\n",
        "  addpoint a at=20,70\n  addpoint b at=180,70 mode=arc rx=90 ry=50 bulge=left\n",
        "  fill none\n",
        "place arch shape=arch at=0,0\n",
    ));
    let f = p.to_str().unwrap();
    let o = run(&[
        "-f",
        f,
        "text-on-path",
        "RIDE THE CURVE",
        "--path",
        "arch",
        "--name",
        "label",
    ]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let svg = String::from_utf8(run(&["-f", f, "inspect", "--svg"]).stdout).unwrap();
    cleanup(&p);
    assert!(svg.contains("<textPath href="), "textPath emitted: {svg}");
    assert!(svg.contains("RIDE THE CURVE"), "content present: {svg}");
}

#[test]
fn text_on_path_missing_path_errors_cleanly() {
    let p = write_temp_strok("documentsize 50x50\n\nshape a template=rectangle\n  fill #000000\nplace a shape=a at=0,0 size=50x50\n");
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "text-on-path", "hi", "--path", "ghost"]);
    cleanup(&p);
    assert!(!o.status.success());
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("ghost"), "names missing path: {e}");
    assert!(!e.contains("panicked"), "must not panic: {e}");
}

#[test]
fn measure_json_schema_is_stable() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape a template=rectangle\n  fill #111111\n",
        "shape b template=rectangle\n  fill #222222\n",
        "place a shape=a at=0,0 size=10x10\n",
        "place b shape=b at=20,0 size=10x10\n",
    ));
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "measure", "a", "b", "--json"]);
    cleanup(&p);
    assert!(o.status.success());
    let json = String::from_utf8(o.stdout).unwrap();
    // Hand-computed: centers (5,5) and (25,5) → dx=20, gap_x=10.
    assert!(json.contains("\"dx\": 20"), "json: {json}");
    assert!(json.contains("\"gap_x\": 10"), "json: {json}");
    assert!(json.contains("\"center_distance\": 20"), "json: {json}");
    assert!(json.contains("\"overlaps\": false"), "json: {json}");
}

#[test]
fn measure_missing_element_errors_cleanly() {
    let p = write_temp_strok("documentsize 50x50\n\nshape a template=rectangle\n  fill #000000\nplace a shape=a at=0,0 size=50x50\n");
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "measure", "a", "ghost"]);
    cleanup(&p);
    assert!(!o.status.success());
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("ghost"), "names missing element: {e}");
    assert!(!e.contains("panicked"), "must not panic: {e}");
}

#[test]
fn snap_grid_rounds_position_and_round_trips() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n\n",
        "shape a template=rectangle\n  fill #000000\n",
        "place a shape=a at=13,27 size=10x10\n",
    ));
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "snap", "a", "grid", "--step", "8"]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let txt = String::from_utf8(run(&["-f", f, "inspect"]).stdout).unwrap();
    cleanup(&p);
    // 13→16, 27→24 on an 8 grid.
    assert!(txt.contains("at=16,24"), "snapped position: {txt}");
}

// ── C6 / E3.1 — diagnostics & error recovery (CLI) ────────────────────

#[test]
fn diagnostic_has_caret_and_suggestion() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n",
        "shape s template=path\n",
        "  storke #ff0000\n",
    ));
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "inspect", "--detail", "summary"]);
    cleanup(&p);
    assert!(!o.status.success(), "bad op should fail");
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("line 3, column 3"), "position: {e}");
    assert!(e.contains("^^^^^^"), "caret: {e}");
    assert!(e.contains("did you mean `stroke`?"), "suggestion: {e}");
    assert!(!e.contains("panicked"), "no panic: {e}");
    // The double-`error:` prefix must not appear (diagnostics own their prefix).
    assert!(!e.contains("error: error:"), "no double prefix: {e}");
}

#[test]
fn addpoint_positional_suggests_at_keyed_form() {
    let p = write_temp_strok(concat!(
        "documentsize 100x100\n",
        "shape s template=path\n",
        "  addpoint a 5,5\n",
    ));
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "inspect", "--detail", "summary"]);
    cleanup(&p);
    assert!(!o.status.success());
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("did you mean `at=5,5`?"), "at= suggestion: {e}");
}

// ── C6 / E3.2 — inspection / query / --json (CLI) ─────────────────────

fn three_box_doc() -> PathBuf {
    write_temp_strok(concat!(
        "documentsize 200x100\n\n",
        "shape box template=rectangle\n  fill #111111\n",
        "place card shape=box at=10,10 size=80x40\n",
        "place badge shape=box at=120,10 size=60x60\n",
        "place inner shape=box at=20,20 size=10x10\n",
    ))
}

#[test]
fn inspect_structural_lists_elements() {
    let p = three_box_doc();
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "inspect", "--detail", "structural"]);
    cleanup(&p);
    assert!(o.status.success());
    let out = String::from_utf8(o.stdout).unwrap();
    assert!(out.contains("card (rectangle) bbox=10,10 80x40"), "{out}");
    assert!(out.contains("3 element(s)"), "{out}");
}

#[test]
fn inspect_json_schema_is_stable() {
    let p = three_box_doc();
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "inspect", "--json"]);
    cleanup(&p);
    assert!(o.status.success());
    let json = String::from_utf8(o.stdout).unwrap();
    assert!(json.contains("\"detail\": \"structural\""), "{json}");
    assert!(json.contains("\"count\": 3"), "{json}");
    assert!(json.contains("\"name\": \"card\""), "{json}");
    assert!(json.contains("\"kind\": \"rectangle\""), "{json}");
    assert!(json.contains("\"x\": 10"), "{json}");
}

#[test]
fn query_box_matches_region() {
    let p = three_box_doc();
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "query", "--box", "0,0,100,100", "--json"]);
    cleanup(&p);
    assert!(o.status.success());
    let json = String::from_utf8(o.stdout).unwrap();
    // card and inner are in [0,0,100,100]; badge (x=120) is not.
    assert!(json.contains("\"query\": \"box 0,0,100,100\""), "{json}");
    assert!(json.contains("\"card\""), "{json}");
    assert!(json.contains("\"inner\""), "{json}");
    assert!(!json.contains("\"badge\""), "badge excluded: {json}");
}

#[test]
fn query_overlaps_finds_overlapping_element() {
    let p = three_box_doc();
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "query", "--overlaps", "card", "--json"]);
    cleanup(&p);
    assert!(o.status.success());
    let json = String::from_utf8(o.stdout).unwrap();
    // inner (20,20 10x10) sits inside card (10,10 80x40); badge does not overlap.
    assert!(json.contains("\"inner\""), "{json}");
    assert!(!json.contains("\"badge\""), "{json}");
}

#[test]
fn relate_reports_spatial_relation() {
    let p = three_box_doc();
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "relate", "card", "badge", "--json"]);
    cleanup(&p);
    assert!(o.status.success());
    let json = String::from_utf8(o.stdout).unwrap();
    assert!(json.contains("\"horizontal\": \"right-of\""), "{json}");
    assert!(json.contains("\"overlaps\": false"), "{json}");
    assert!(json.contains("\"top\""), "aligned top edge: {json}");
}

#[test]
fn relate_missing_element_errors_cleanly() {
    let p = three_box_doc();
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "relate", "card", "ghost"]);
    cleanup(&p);
    assert!(!o.status.success());
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("ghost"), "{e}");
    assert!(!e.contains("panicked"), "{e}");
}

#[test]
fn render_annotate_overlays_ids() {
    let p = three_box_doc();
    let f = p.to_str().unwrap();
    let out_png = temp_path("png");
    let o = run(&[
        "-f",
        f,
        "render",
        "--annotate",
        "--out",
        out_png.to_str().unwrap(),
    ]);
    let produced = out_png.exists() && fs::metadata(&out_png).map(|m| m.len() > 0).unwrap_or(false);
    cleanup(&p);
    cleanup(&out_png);
    assert!(
        o.status.success(),
        "annotate render failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(produced, "annotate render produced a non-empty PNG");
}

#[test]
fn render_outline_accepts_bare_and_selected_forms() {
    let p = three_box_doc();
    let f = p.to_str().unwrap();
    let all_png = temp_path("png");
    let selected_png = temp_path("png");

    let all = run(&[
        "-f",
        f,
        "render",
        "--outline",
        "--out",
        all_png.to_str().unwrap(),
    ]);
    assert!(
        all.status.success(),
        "bare outline render failed: {}",
        String::from_utf8_lossy(&all.stderr)
    );

    let selected = run(&[
        "-f",
        f,
        "render",
        "--outline",
        "card,inner",
        "--region",
        "0,0,100,100",
        "--width",
        "400",
        "--out",
        selected_png.to_str().unwrap(),
    ]);
    assert!(
        selected.status.success(),
        "selected outline render failed: {}",
        String::from_utf8_lossy(&selected.stderr)
    );

    let all_bytes = fs::read(&all_png).expect("read all-outline PNG");
    let selected_bytes = fs::read(&selected_png).expect("read selected-outline PNG");
    assert!(!all_bytes.is_empty());
    assert!(!selected_bytes.is_empty());
    assert_eq!(png_dimensions(&selected_bytes), (400, 400));
    assert_ne!(
        all_bytes, selected_bytes,
        "selected outline + region must produce a distinct render"
    );

    cleanup(&p);
    cleanup(&all_png);
    cleanup(&selected_png);
}

#[test]
fn render_outline_rejects_unknown_and_empty_ids() {
    let p = three_box_doc();
    let f = p.to_str().unwrap();

    let unknown = run(&["-f", f, "render", "--outline", "ghost"]);
    assert!(!unknown.status.success());
    let unknown_err = String::from_utf8_lossy(&unknown.stderr);
    assert!(
        unknown_err.contains("outline id 'ghost'") && unknown_err.contains("not a placed element"),
        "{unknown_err}"
    );

    let empty = run(&["-f", f, "render", "--outline="]);
    assert!(!empty.status.success());
    let empty_err = String::from_utf8_lossy(&empty.stderr);
    assert!(
        empty_err.contains("--outline expects comma-separated placed IDs"),
        "{empty_err}"
    );
    assert!(!empty_err.contains("panicked"), "{empty_err}");

    cleanup(&p);
}

// --- C7: visual diff (E3.3) -------------------------------------------------

/// Render a tiny doc to PNG via the CLI and return the output path.
fn render_doc_png(doc: &str) -> (PathBuf, PathBuf) {
    let p = write_temp_strok(doc);
    let png = temp_path("png");
    let o = run(&[
        "-f",
        p.to_str().unwrap(),
        "render",
        "--width",
        "48",
        "--height",
        "48",
        "--bg",
        "#ffffff",
        "--out",
        png.to_str().unwrap(),
    ]);
    assert!(
        o.status.success(),
        "render failed: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    (p, png)
}

#[test]
fn diff_identical_images_is_within_tolerance() {
    let (src, png) = render_doc_png(concat!(
        "documentsize 48x48\n",
        "shape b template=rectangle\n  fill #000000\n",
        "place b shape=b at=8,8 size=20x20\n",
    ));
    let o = run(&["diff", png.to_str().unwrap(), png.to_str().unwrap()]);
    cleanup(&src);
    cleanup(&png);
    // Identical images: within tolerance => exit 0.
    assert!(
        o.status.success(),
        "identical diff should exit 0: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    assert!(String::from_utf8_lossy(&o.stderr).contains("no changes"));
}

#[test]
fn diff_highlights_a_known_change_region() {
    // Two docs differing only in the location of a black box → the diff must
    // flag a change and (with --out) write a diff PNG.
    let (src_a, png_a) = render_doc_png(concat!(
        "documentsize 48x48\n",
        "shape b template=rectangle\n  fill #000000\n",
        "place b shape=b at=4,4 size=12x12\n",
    ));
    let (src_b, png_b) = render_doc_png(concat!(
        "documentsize 48x48\n",
        "shape b template=rectangle\n  fill #000000\n",
        "place b shape=b at=30,30 size=12x12\n",
    ));
    let diff_png = temp_path("png");
    let o = run(&[
        "diff",
        png_a.to_str().unwrap(),
        png_b.to_str().unwrap(),
        "--out",
        diff_png.to_str().unwrap(),
        "--json",
    ]);
    let produced = diff_png.exists()
        && fs::metadata(&diff_png)
            .map(|m| m.len() > 0)
            .unwrap_or(false);
    let stdout = String::from_utf8_lossy(&o.stdout).to_string();
    let exit_ok = o.status.success();
    cleanup(&src_a);
    cleanup(&src_b);
    cleanup(&png_a);
    cleanup(&png_b);
    cleanup(&diff_png);
    assert!(produced, "diff PNG was written");
    assert!(stdout.contains("\"changed_pixels\""), "{stdout}");
    assert!(stdout.contains("\"changed_bbox\""), "{stdout}");
    // A real difference => not within tolerance => non-zero exit.
    assert!(!exit_ok, "differing images should exit non-zero");
    assert!(stdout.contains("\"within_tolerance\": false"), "{stdout}");
}

#[test]
fn diff_rejects_mixing_files_and_since() {
    let p = write_temp_strok("documentsize 24x24\n");
    let o = run(&[
        "-f",
        p.to_str().unwrap(),
        "diff",
        "a.png",
        "b.png",
        "--since",
        "1",
    ]);
    cleanup(&p);
    assert!(!o.status.success());
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("not both") || e.contains("either"), "{e}");
    assert!(!e.contains("panicked"), "{e}");
}

#[test]
fn diff_since_on_dsl_doc_explains_no_history() {
    // v3 DSL files don't persist the op log, so --since explains the limitation
    // rather than crashing (honesty register).
    let p = write_temp_strok(concat!(
        "documentsize 24x24\n",
        "shape b template=rectangle\n  fill #000000\n",
        "place b shape=b at=0,0 size=24x24\n",
    ));
    let o = run(&["-f", p.to_str().unwrap(), "diff", "--since", "1"]);
    cleanup(&p);
    assert!(!o.status.success());
    let e = String::from_utf8_lossy(&o.stderr);
    assert!(e.contains("history"), "{e}");
    assert!(!e.contains("panicked"), "{e}");
}

// --- C7: MCP server (E3.4) --------------------------------------------------

/// Drive the MCP stdio server with a sequence of JSON-RPC lines; return stdout.
fn run_mcp(input: &str) -> String {
    use std::io::Write;
    use std::process::Stdio;
    let mut child = Command::new(env!("CARGO_BIN_EXE_strok"))
        .arg("mcp-server")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("spawn mcp-server");
    child
        .stdin
        .as_mut()
        .expect("stdin")
        .write_all(input.as_bytes())
        .expect("write stdin");
    let out = child.wait_with_output().expect("wait mcp-server");
    String::from_utf8_lossy(&out.stdout).to_string()
}

#[test]
fn mcp_smoke_create_render_inspect() {
    // initialize → tools/call new → render → inspect, all over stdio.
    let src = "documentsize 24x24\\nshape b template=rectangle\\n  fill #000000\\nplace b shape=b at=0,0 size=24x24";
    let input = format!(
        concat!(
            r#"{{"jsonrpc":"2.0","id":1,"method":"initialize","params":{{}}}}"#,
            "\n",
            r#"{{"jsonrpc":"2.0","method":"notifications/initialized"}}"#,
            "\n",
            r#"{{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{{"name":"render","arguments":{{"source":"{src}"}}}}}}"#,
            "\n",
            r#"{{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{{"name":"inspect","arguments":{{"source":"{src}","detail":"summary"}}}}}}"#,
            "\n",
        ),
        src = src
    );
    let out = run_mcp(&input);
    let lines: Vec<&str> = out.lines().collect();
    // 3 responses (the notification produces none).
    assert_eq!(lines.len(), 3, "got: {out}");
    assert!(lines[0].contains("serverInfo"), "{}", lines[0]);
    assert!(lines[1].contains(r#""type":"image""#), "{}", lines[1]);
    assert!(lines[1].contains("image/png"), "{}", lines[1]);
    // The inspect JSON is carried as escaped text inside the JSON-RPC string,
    // so its quotes appear as \" — match the escaped form.
    assert!(lines[2].contains(r#"isError\":false"#) || lines[2].contains("isError"));
    assert!(lines[2].contains("elements"), "{}", lines[2]);
}

// ── C10: sprites / contact sheet / manifest / token-sync / audit --json ──

fn scratch_dir(tag: &str) -> PathBuf {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    let seq = TEMP_SEQ.fetch_add(1, Ordering::Relaxed);
    let dir = std::env::temp_dir().join(format!(
        "strok-{}-{}-{}-{}",
        tag,
        std::process::id(),
        nanos,
        seq
    ));
    fs::create_dir_all(&dir).unwrap();
    dir
}

const C10_CLOSE_ICON: &str = "\
# @meaning Close or dismiss the current view
# @tags close, dismiss, x
documentsize 24x24

defaults
  fill none
  stroke currentColor
  stroke-width 2

shape x template=path
  addpoint a at=6,6
  addpoint b at=18,18

place x shape=x at=0,0
";

const C10_BACK_ICON: &str = "\
# @meaning Go back to the previous view
# @tags back, previous, arrow
documentsize 24x24

defaults
  fill none
  stroke currentColor
  stroke-width 2

shape a template=path
  addpoint p at=14,6
  addpoint q at=8,12
  addpoint r at=14,18

place a shape=a at=0,0
";

#[test]
fn batch_sprite_sheet_emits_symbols_and_keeps_currentcolor() {
    let dir = scratch_dir("c10-sprite");
    fs::write(dir.join("close.strok"), C10_CLOSE_ICON).unwrap();
    fs::write(dir.join("arrow-left.strok"), C10_BACK_ICON).unwrap();
    let sprite = dir.join("sprite.svg");

    let o = run(&[
        "batch",
        dir.to_str().unwrap(),
        "--svg",
        "--sprite",
        sprite.to_str().unwrap(),
    ]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );

    let svg = fs::read_to_string(&sprite).expect("sprite.svg");
    // One <symbol> per icon, id = file stem, sorted, viewBox carried.
    assert!(
        svg.contains(r#"<symbol id="arrow-left" viewBox="0 0 24 24">"#),
        "{svg}"
    );
    assert!(
        svg.contains(r#"<symbol id="close" viewBox="0 0 24 24">"#),
        "{svg}"
    );
    // arrow-left sorts before close.
    assert!(svg.find("arrow-left").unwrap() < svg.find("\"close\"").unwrap());
    // Themeable: currentColor survives into the sprite.
    assert!(svg.contains("currentColor"), "{svg}");
    assert!(svg.trim_end().ends_with("</svg>"));

    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn batch_contact_sheet_writes_grid_png() {
    let dir = scratch_dir("c10-sheet");
    fs::write(dir.join("close.strok"), C10_CLOSE_ICON).unwrap();
    fs::write(dir.join("arrow-left.strok"), C10_BACK_ICON).unwrap();
    let sheet = dir.join("contact.png");

    let o = run(&[
        "batch",
        dir.to_str().unwrap(),
        "--sheet",
        sheet.to_str().unwrap(),
        "--columns",
        "2",
        "--sizes",
        "24",
        "--color",
        "#1a1a1a",
    ]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let bytes = fs::read(&sheet).expect("contact.png");
    // PNG magic.
    assert_eq!(&bytes[..8], b"\x89PNG\r\n\x1a\n");
    assert!(bytes.len() > 100);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn batch_manifest_schema_is_stable() {
    let dir = scratch_dir("c10-manifest");
    fs::write(dir.join("close.strok"), C10_CLOSE_ICON).unwrap();
    fs::write(dir.join("arrow-left.strok"), C10_BACK_ICON).unwrap();
    let manifest = dir.join("manifest.json");

    let o = run(&[
        "batch",
        dir.to_str().unwrap(),
        "--svg",
        "--manifest",
        manifest.to_str().unwrap(),
        "--sizes",
        "16,24",
    ]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let json = fs::read_to_string(&manifest).expect("manifest.json");
    // Normalize the volatile nothing (paths absent), snapshot the schema.
    insta::assert_snapshot!("c10_manifest", json);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn token_sync_flags_undefined_reference_and_exits_nonzero() {
    let system = scratch_dir("c10-ts-sys");
    let sys_file = system.join("ds.strok");
    fs::write(
        &sys_file,
        "documentsize 24x24\npalette\n  copper #b87333\ntokens\n  color.ink #1a1a1a\n  space.md 16\n",
    )
    .unwrap();

    let icons = scratch_dir("c10-ts-icons");
    fs::write(
        icons.join("good.strok"),
        "documentsize 24x24\nshape s template=rectangle\n  fill $copper\n  stroke $color.ink\nplace s shape=s at=0,0\n",
    )
    .unwrap();
    fs::write(
        icons.join("bad.strok"),
        "documentsize 24x24\nshape s template=rectangle\n  fill $nope\nplace s shape=s at=0,0\n",
    )
    .unwrap();

    let o = run(&[
        "token-sync",
        icons.to_str().unwrap(),
        "--system",
        sys_file.to_str().unwrap(),
        "--json",
    ]);
    // Undefined token => non-zero exit.
    assert!(!o.status.success());
    let json = String::from_utf8(o.stdout).unwrap();
    assert!(json.contains("\"in_sync\": false"), "{json}");
    assert!(json.contains("\"nope\""), "{json}");
    // copper (bare color) + color.ink (dotted) both resolve.
    assert!(json.contains("\"copper\""), "{json}");
    assert!(json.contains("\"color.ink\""), "{json}");
    // space.md is defined but unused.
    assert!(json.contains("\"space.md\""), "{json}");

    let _ = fs::remove_dir_all(&system);
    let _ = fs::remove_dir_all(&icons);
}

#[test]
fn audit_json_schema_is_stable_and_actionable() {
    let p = write_temp_strok(
        "documentsize 400x400\n\nshape eye-l template=ellipse\n  movepoint top to=120,80\n  movepoint bottom to=120,100\n  movepoint left to=110,90\n  movepoint right to=130,90\n  fill #3a2510\n\nshape eye-r template=ellipse\n  movepoint top to=280,80\n  movepoint bottom to=280,100\n  movepoint left to=270,90\n  movepoint right to=290,90\n  fill #3a2510\n\nplace eye-l shape=eye-l at=0,0 size=40x30\nplace eye-r shape=eye-r at=0,0 size=40x30\n",
    );
    let f = p.to_str().unwrap();
    let o = run(&["-f", f, "audit", "--json"]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let json = String::from_utf8(o.stdout).unwrap();
    assert!(json.contains("\"kind\": \"near_mirror\""), "{json}");
    assert!(json.contains("\"suggestion\":"), "{json}");
    assert!(json.contains("flip=x"), "{json}");
    assert!(json.contains("\"count\":"), "{json}");
    assert!(json.contains("\"total_line_savings\":"), "{json}");
    cleanup(&p);
}

#[test]
fn audit_reports_partial_text_collision_with_anchor_rewrite() {
    let p = write_temp_strok(
        "documentsize 400x300\n\nshape badge template=ellipse\n  fill #1248cf\n\nshape label template=text\n  content \"RUNTIME A\"\n  font-size 20\n  font-weight 800\n\nplace badge shape=badge at=220,100 size=60x60\nplace label shape=label at=185,135\n",
    );
    let f = p.to_str().unwrap();
    let output = run(&["-f", f, "audit"]);
    assert!(
        output.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    let stderr = String::from_utf8(output.stderr).unwrap();
    assert!(stderr.contains("text-collision"), "{stderr}");
    assert!(stderr.contains("at=badge.left"), "{stderr}");
    assert!(stderr.contains("align=right"), "{stderr}");
    cleanup(&p);
}

// ── EXP-1: embedded standard shape library (`strok lib`) ─────────────────

#[test]
fn lib_list_prints_every_module_and_shape() {
    let o = run(&["lib", "list"]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let out = String::from_utf8(o.stdout).unwrap();
    for module in ["figures", "arrows", "bubbles", "devices", "furniture"] {
        assert!(out.contains(&format!("std/{}", module)), "{out}");
    }
    assert!(out.contains("person-standing"), "{out}");
    assert!(out.contains("arrow-right"), "{out}");
}

#[test]
fn lib_list_json_schema_is_stable() {
    let o = run(&["lib", "list", "--json"]);
    assert!(o.status.success());
    let json = String::from_utf8(o.stdout).unwrap();
    assert!(json.contains("\"module\": \"figures\""), "{json}");
    assert!(json.contains("\"shapes\":"), "{json}");
    assert!(json.contains("\"name\": \"person-standing\""), "{json}");
    assert!(json.contains("\"meaning\":"), "{json}");
    assert!(json.contains("\"tags\":"), "{json}");
}

#[test]
fn lib_show_prints_module_source() {
    let o = run(&["lib", "show", "figures"]);
    assert!(
        o.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&o.stderr)
    );
    let out = String::from_utf8(o.stdout).unwrap();
    assert!(out.contains("shape person-standing template=path"), "{out}");
    assert!(out.contains("# @meaning"), "{out}");
}

#[test]
fn lib_show_accepts_std_prefixed_name() {
    let o = run(&["lib", "show", "std/arrows"]);
    assert!(o.status.success());
    let out = String::from_utf8(o.stdout).unwrap();
    assert!(out.contains("shape arrow-right"), "{out}");
}

#[test]
fn lib_show_unknown_module_errors_cleanly() {
    let o = run(&["lib", "show", "nope"]);
    assert!(!o.status.success());
    let err = String::from_utf8(o.stderr).unwrap();
    assert!(err.contains("unknown standard library module"), "{err}");
    assert!(err.contains("figures"), "{err}"); // lists available modules
}

#[test]
fn lib_search_matches_name_meaning_and_tags() {
    let o = run(&["lib", "search", "person"]);
    assert!(o.status.success());
    let out = String::from_utf8(o.stdout).unwrap();
    assert!(out.contains("person-standing"), "{out}");
    assert!(out.contains("person-pointing"), "{out}");

    let o2 = run(&["lib", "search", "chat"]); // matches speech-bubble's tags
    assert!(o2.status.success());
    let out2 = String::from_utf8(o2.stdout).unwrap();
    assert!(out2.contains("speech-bubble"), "{out2}");
}

#[test]
fn lib_search_json_schema_is_stable() {
    let o = run(&["lib", "search", "arrow", "--json"]);
    assert!(o.status.success());
    let json = String::from_utf8(o.stdout).unwrap();
    assert!(json.contains("\"module\": \"arrows\""), "{json}");
    assert!(json.contains("\"name\": \"arrow-right\""), "{json}");
}

#[test]
fn std_import_renders_and_round_trips() {
    let strok_path = write_temp_strok(concat!(
        "documentsize 200x200\n",
        "\n",
        "use \"std/figures\" as fig\n",
        "\n",
        "place p shape=fig.person-standing at=10,10 size=40x100\n",
        "  fill #2d5a1e\n",
    ));

    // Renders without error.
    let png_out = temp_path("png");
    let render = run(&[
        "-f",
        strok_path.to_str().unwrap(),
        "render",
        "--out",
        png_out.to_str().unwrap(),
    ]);
    assert!(
        render.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&render.stderr)
    );
    assert!(png_out.exists());
    cleanup(&png_out);

    // The std import resolves into an actual placed path (not empty).
    let svg_out = run(&["-f", strok_path.to_str().unwrap(), "inspect", "--svg"]);
    assert!(svg_out.status.success());
    let svg = String::from_utf8(svg_out.stdout).unwrap();
    assert!(svg.contains("id=\"p\""), "{svg}");
    assert!(svg.contains("fill=\"#2d5a1e\""), "{svg}");

    // Round-trip: the `use` import stays a `use` line, not an inlined shape
    // dump — re-emitting must not duplicate the whole std module into the
    // file (see Scene::imported_shape_names). Trigger a re-emit by adding
    // another place via the CLI (read-modify-write).
    let placed = run(&[
        "-f",
        strok_path.to_str().unwrap(),
        "place",
        "p2",
        "--shape",
        "fig.person-pointing",
        "--at",
        "60,10",
        "--size",
        "40x100",
    ]);
    assert!(
        placed.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&placed.stderr)
    );
    let contents = fs::read_to_string(&strok_path).unwrap();
    assert_eq!(
        contents.matches("shape person-standing").count(),
        0,
        "std module shapes must not be inlined into the document:\n{contents}"
    );
    assert_eq!(contents.matches("use \"std/figures\" as fig").count(), 1);

    cleanup(&strok_path);
}

#[test]
fn std_import_unknown_module_errors_with_available_list() {
    let strok_path = write_temp_strok(concat!(
        "documentsize 100x100\n",
        "\n",
        "use \"std/nope\" as x\n",
    ));
    let o = run(&["-f", strok_path.to_str().unwrap(), "inspect", "--svg"]);
    cleanup(&strok_path);
    assert!(!o.status.success());
    let err = String::from_utf8(o.stderr).unwrap();
    assert!(err.contains("std/nope"), "{err}");
    assert!(err.contains("figures"), "{err}");
}

// --- watch mode -------------------------------------------------------------

/// One raw HTTP GET against the watch server, returning the full response.
fn watch_http_get(port: u16, path: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(
        stream,
        "GET {} HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        path
    )
    .unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    out
}

/// One form-encoded HTTP POST against the watch server.
fn watch_http_post(port: u16, path: &str, body: &str) -> String {
    use std::io::{Read, Write};
    let mut stream = std::net::TcpStream::connect(("127.0.0.1", port)).expect("connect");
    write!(
        stream,
        "POST {} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Type: application/x-www-form-urlencoded\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path,
        body.len(),
        body
    )
    .unwrap();
    let mut out = String::new();
    stream.read_to_string(&mut out).unwrap();
    out
}

#[test]
fn watch_serves_preview_and_rerenders_on_change() {
    use std::io::BufRead;

    let strok_path = write_temp_strok(concat!(
        "documentsize 100x100\n",
        "\n",
        "shape s template=rectangle\n",
        "  fill #ff0000\n",
        "\n",
        "place p shape=s at=10,10 size=50x50\n",
    ));

    // --port 0 binds an ephemeral port; the startup line on stderr reports it.
    let mut child = Command::new(env!("CARGO_BIN_EXE_strok"))
        .args([
            "watch",
            strok_path.to_str().unwrap(),
            "--no-open",
            "--port",
            "0",
        ])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn watch");
    let stderr = child.stderr.take().unwrap();
    let mut lines = std::io::BufReader::new(stderr).lines();
    let port: u16 = loop {
        let line = lines
            .next()
            .expect("watch exited before startup line")
            .unwrap();
        if let Some(idx) = line.find("127.0.0.1:") {
            let rest = &line[idx + "127.0.0.1:".len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            break digits.parse().unwrap();
        }
    };

    let index = watch_http_get(port, "/");
    assert!(index.contains("200 OK"), "{index}");
    assert!(index.contains("text/html"), "{index}");
    assert!(index.contains("Equalize handles"), "{index}");
    assert!(index.contains("Handles stay linked by default"), "{index}");
    assert!(index.contains("id=\"undobtn\""), "{index}");
    assert!(index.contains("id=\"zoomfit\""), "{index}");
    assert!(index.contains("src=\"/watch.js\""), "{index}");

    let css = watch_http_get(port, "/watch.css");
    assert!(css.contains("200 OK"), "{css}");
    assert!(css.contains("text/css"), "{css}");
    assert!(css.contains(".zoom-controls"), "{css}");

    let js = watch_http_get(port, "/watch.js");
    assert!(js.contains("200 OK"), "{js}");
    assert!(js.contains("text/javascript"), "{js}");
    assert!(js.contains("function zoomAt("), "{js}");
    assert!(js.contains("function panViewport("), "{js}");
    assert!(js.contains("function installShapeTargets("), "{js}");

    let viewport = watch_http_get(port, "/viewport.js");
    assert!(viewport.contains("200 OK"), "{viewport}");
    assert!(viewport.contains("class ViewBoxCamera"), "{viewport}");

    let geometry = watch_http_get(port, "/path-geometry.js");
    assert!(geometry.contains("200 OK"), "{geometry}");
    assert!(geometry.contains("function buildPath"), "{geometry}");

    let selection = watch_http_get(port, "/selection.js");
    assert!(selection.contains("200 OK"), "{selection}");
    assert!(selection.contains("class PointSelection"), "{selection}");

    let state = watch_http_get(port, "/state.json");
    assert!(state.contains("\"version\":1"), "{state}");
    assert!(state.contains("#ff0000"), "{state}");
    assert!(state.contains("\"error\":null"), "{state}");

    // Edit the file; the poller (150ms) should re-render and bump the version.
    let mut contents = fs::read_to_string(&strok_path).unwrap();
    contents = contents.replace("#ff0000", "#00cc00");
    fs::write(&strok_path, &contents).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let updated = loop {
        let state = watch_http_get(port, "/state.json");
        if state.contains("#00cc00") {
            break state;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "watch never picked up the edit; last state: {state}"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    };
    assert!(updated.contains("\"version\":2"), "{updated}");

    // Break the file: the error is surfaced but the last good render is kept.
    fs::write(&strok_path, format!("{contents}garbage line\n")).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let state = watch_http_get(port, "/state.json");
        if state.contains("unexpected top-level keyword") {
            assert!(state.contains("#00cc00"), "last good svg dropped: {state}");
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "watch never reported the parse error; last state: {state}"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    child.kill().unwrap();
    let _ = child.wait();
    cleanup(&strok_path);
}

#[test]
fn watch_visually_edits_points_controls_and_topology() {
    use std::io::BufRead;

    let strok_path = write_temp_strok(concat!(
        "documentsize 100x100\n",
        "\n",
        "shape curve template=path\n",
        "  addpoint a at=10,10\n",
        "  addpoint b at=60,20 mode=controls c1=25,5 c2=50,15\n",
        "  addpoint c at=70,70\n",
        "  addpoint d at=10,70\n",
        "  close\n",
        "  fill #ff0000\n",
        "\n",
        "place curve-preview shape=curve at=0,0\n",
    ));

    let mut child = Command::new(env!("CARGO_BIN_EXE_strok"))
        .args([
            "watch",
            strok_path.to_str().unwrap(),
            "--no-open",
            "--port",
            "0",
        ])
        .stderr(std::process::Stdio::piped())
        .spawn()
        .expect("spawn watch");
    let stderr = child.stderr.take().unwrap();
    let mut lines = std::io::BufReader::new(stderr).lines();
    let port: u16 = loop {
        let line = lines
            .next()
            .expect("watch exited before startup line")
            .unwrap();
        if let Some(idx) = line.find("127.0.0.1:") {
            let rest = &line[idx + "127.0.0.1:".len()..];
            let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            break digits.parse().unwrap();
        }
    };

    let state = watch_http_get(port, "/state.json");
    assert!(state.contains("\"editor\":[{\"name\":\"curve\""), "{state}");
    assert!(state.contains("\"controlsEditable\":true"), "{state}");
    assert!(state.contains("\"canSymmetrize\":true"), "{state}");
    assert!(state.contains("\"canUndo\":false"), "{state}");
    assert!(
        state.contains("\"targets\":[{\"name\":\"curve-preview\",\"shape\":\"curve\",\"transform\":[1,0,0,1,0,0]}]"),
        "{state}"
    );

    let moved = watch_http_post(port, "/edit", "action=move&shape=curve&point=a&x=12&y=14");
    assert!(moved.contains("200 OK"), "{moved}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(source.contains("addpoint a at=12,14"), "{source}");

    let nudged = watch_http_post(port, "/edit", "action=nudge&shape=curve&point=d&dx=1&dy=-2");
    assert!(nudged.contains("200 OK"), "{nudged}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(source.contains("addpoint d at=11,68"), "{source}");

    let controlled = watch_http_post(
        port,
        "/edit",
        "action=control&shape=curve&point=b&handle=c1&x=30&y=6",
    );
    assert!(controlled.contains("200 OK"), "{controlled}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(source.contains("c1=30,6"), "{source}");

    let symmetric = watch_http_post(port, "/edit", "action=symmetric&shape=curve&point=a");
    assert!(symmetric.contains("200 OK"), "{symmetric}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(
        source.contains("addpoint a at=12,14 mode=controls"),
        "{source}"
    );

    let linked = watch_http_post(
        port,
        "/edit",
        "action=control&shape=curve&point=b&handle=c1&x=25&y=14&oppositePoint=a&oppositeHandle=c2&oppositeX=5&oppositeY=14",
    );
    assert!(linked.contains("200 OK"), "{linked}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(source.contains("c1=25,14"), "{source}");
    assert!(source.contains("c2=5,14"), "{source}");

    let retracted = watch_http_post(
        port,
        "/edit",
        "action=retract-control&shape=curve&point=b&handle=c1",
    );
    assert!(retracted.contains("200 OK"), "{retracted}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(source.contains("c1=12,14"), "{source}");

    let undone = watch_http_post(port, "/edit", "action=undo");
    assert!(undone.contains("200 OK"), "{undone}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(source.contains("c1=25,14"), "{source}");
    let redone = watch_http_post(port, "/edit", "action=redo");
    assert!(redone.contains("200 OK"), "{redone}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(source.contains("c1=12,14"), "{source}");

    // A hand edit remains authoritative and invalidates incompatible browser
    // history rather than allowing an undo to overwrite it later.
    fs::write(&strok_path, source.replace("#ff0000", "#00ff00")).unwrap();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    loop {
        let state = watch_http_get(port, "/state.json");
        if state.contains("#00ff00") && state.contains("\"canUndo\":false") {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "external edit did not clear browser history: {state}"
        );
        std::thread::sleep(std::time::Duration::from_millis(100));
    }

    let added = watch_http_post(port, "/edit", "action=add&shape=curve&after=a");
    assert!(added.contains("200 OK"), "{added}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(source.contains("addpoint p1"), "{source}");
    assert!(source.contains("after=a"), "{source}");

    let deleted = watch_http_post(port, "/edit", "action=delete&shape=curve&point=p1");
    assert!(deleted.contains("200 OK"), "{deleted}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(source.contains("deletepoint p1"), "{source}");

    let moved = watch_http_post(
        port,
        "/edit",
        "action=move-anchors&shape=curve&points=c,d&dx=2&dy=-3",
    );
    assert!(moved.contains("200 OK"), "{moved}");
    let source = fs::read_to_string(&strok_path).unwrap();
    assert!(source.contains("addpoint c at=72,67"), "{source}");
    assert!(source.contains("addpoint d at=13,65"), "{source}");

    let duplicate = watch_http_post(
        port,
        "/edit",
        "action=move-anchors&shape=curve&points=c,c&dx=1&dy=0",
    );
    assert!(duplicate.contains("400 Bad Request"), "{duplicate}");
    let unchanged = fs::read_to_string(&strok_path).unwrap();
    assert_eq!(source, unchanged);

    let missing = watch_http_post(
        port,
        "/edit",
        "action=move-anchors&shape=curve&points=c,missing&dx=1&dy=0",
    );
    assert!(missing.contains("400 Bad Request"), "{missing}");
    let unchanged = fs::read_to_string(&strok_path).unwrap();
    assert_eq!(source, unchanged);

    child.kill().unwrap();
    let _ = child.wait();
    cleanup(&strok_path);
}
