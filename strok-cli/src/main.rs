mod cli;
mod mcp;
mod watch;

use anyhow::{Context, Result};
use clap::Parser;
use std::collections::BTreeMap;
use std::io::Write;
use std::path::Path;
use strok_core::audit;
use strok_core::document::Document;
use strok_core::dsl_emit;
use strok_core::dsl_parse;
use strok_core::emit;
use strok_core::json::Json;
use strok_core::manifest::{self, IconEntry, Manifest, SpriteSymbol};
use strok_core::resolve;
use strok_core::scene::Scene;
use strok_core::stdlib;
use strok_core::token_sync;
use strok_render::{
    contact_sheet, render_to_png, target_dimensions, RenderOptions, RenderRegion, SheetOptions,
    SheetTile,
};

use cli::{Cli, Command, LibAction};

fn main() {
    if let Err(e) = run() {
        // Positioned parse diagnostics (E3.1) already render their own
        // `error: …` lines with caret snippets — print them verbatim instead of
        // double-prefixing. Other errors get the plain `error:` prefix.
        if let Some(strok_core::error::StrokError::ParseDiagnostics(diags)) =
            e.downcast_ref::<strok_core::error::StrokError>()
        {
            for (i, d) in diags.iter().enumerate() {
                if i > 0 {
                    eprintln!();
                }
                eprintln!("{}", d.render());
            }
        } else {
            eprintln!("error: {:#}", e);
        }
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let cli = Cli::parse();

    match &cli.command {
        Command::AgentIntro => {
            print!("{AGENT_INTRO}");
        }

        Command::Guide { topic } => {
            print!("{}", guide_text(topic)?);
        }

        Command::New {
            path,
            size,
            profile,
        } => {
            let contents = build_document_source(size, profile.as_deref())
                .map_err(|e| anyhow::anyhow!("invalid document for '{}': {}", path, e))?;
            std::fs::write(path, &contents)?;
            eprintln!("created {}", path);
            if profile.as_deref() == Some("icon") {
                eprintln!(
                    "note: `icon` is the round-outline compatibility alias; for a new set, run `strok guide icon` and choose an explicit icon-* profile"
                );
            }
        }

        Command::McpServer => {
            mcp::run_stdio()?;
        }

        // Watch takes its file positionally (like `new`) or via -f.
        Command::Watch {
            file,
            port,
            scheme,
            no_open,
        } => {
            let file = file.as_deref().or(cli.file.as_deref()).ok_or_else(|| {
                anyhow::anyhow!(
                    "missing file to watch\n\nUsage: strok watch <file>   (or strok -f <file> watch)"
                )
            })?;
            watch::run(Path::new(file), *port, scheme.as_deref(), !no_open)?;
        }

        // `diff a.png b.png` needs no document file; `diff --since N` does and is
        // dispatched in the file-bound arm below.
        Command::Diff {
            a: Some(a),
            b: Some(b),
            since: None,
            out,
            json,
            ..
        } => {
            diff_images(a, b, out.as_deref(), *json)?;
        }

        Command::Batch {
            dir,
            out,
            svg,
            png,
            sizes,
            color,
            bg,
            scheme,
            sprite,
            sheet,
            manifest,
            columns,
        } => {
            batch_render(BatchArgs {
                dir,
                out: out.as_deref(),
                svg: *svg,
                png: *png,
                sizes: sizes.as_deref(),
                color: color.as_deref(),
                bg: bg.as_deref(),
                scheme: scheme.as_deref(),
                sprite: sprite.as_deref(),
                sheet: sheet.as_deref(),
                manifest: manifest.as_deref(),
                columns: *columns,
            })?;
        }

        Command::TokenSync { dir, system, json } => {
            token_sync_cmd(dir, system, *json)?;
        }

        Command::Import { input, out, json } => {
            import_svg_cmd(input, out, *json)?;
        }

        Command::Lib { action } => {
            lib_cmd(action)?;
        }

        _ => {
            let file = cli.file.as_deref().ok_or_else(|| {
                anyhow::anyhow!(
                    "missing -f <file>\n\nUsage: strok -f <file> <command>\n\nTo create a new file: strok new <path> 800x800"
                )
            })?;
            let path = Path::new(file);
            if !path.exists() {
                anyhow::bail!(
                    "'{}': file not found\n\nCreate it with: strok new {} 800x800",
                    file,
                    file
                );
            }

            match &cli.command {
                Command::New { .. }
                | Command::Batch { .. }
                | Command::TokenSync { .. }
                | Command::Import { .. }
                | Command::Lib { .. }
                | Command::AgentIntro
                | Command::Guide { .. }
                | Command::McpServer
                | Command::Watch { .. } => {
                    unreachable!()
                }

                Command::Diff {
                    a,
                    b,
                    since,
                    out,
                    width,
                    height,
                    color,
                    json,
                } => {
                    diff_since(
                        path,
                        a.as_deref(),
                        b.as_deref(),
                        *since,
                        out.as_deref(),
                        *width,
                        *height,
                        color.as_deref(),
                        *json,
                    )?;
                }

                Command::Exec { line } => {
                    append_line(path, line)?;
                }

                Command::Shape {
                    name,
                    template,
                    ops,
                } => {
                    let mut block = format!("shape {} template={}\n", name, template);
                    for op in ops {
                        block.push_str(&format!("  {}\n", op));
                    }
                    append_line(path, &block)?;
                }

                Command::Place {
                    name,
                    shape,
                    at,
                    size,
                    rotation,
                    flip,
                    align,
                    offset,
                    from,
                    to,
                    center,
                    radius,
                } => {
                    let mut line = format!("place {} shape={}", name, shape);
                    if let Some(at) = at {
                        line.push_str(&format!(" at={}", at));
                    }
                    if let Some(size) = size {
                        line.push_str(&format!(" size={}", size));
                    }
                    if let Some(from) = from {
                        line.push_str(&format!(" from={}", from));
                    }
                    if let Some(to) = to {
                        line.push_str(&format!(" to={}", to));
                    }
                    if let Some(center) = center {
                        line.push_str(&format!(" center={}", center));
                    }
                    if let Some(radius) = radius {
                        line.push_str(&format!(" radius={}", radius));
                    }
                    if let Some(rot) = rotation {
                        if rot.ends_with("deg") {
                            line.push_str(&format!(" rotation={}", rot));
                        } else {
                            line.push_str(&format!(" rotation={}deg", rot));
                        }
                    }
                    if let Some(flip) = flip {
                        line.push_str(&format!(" flip={}", flip));
                    }
                    if let Some(align) = align {
                        line.push_str(&format!(" align={}", align));
                    }
                    if let Some(offset) = offset {
                        line.push_str(&format!(" offset={}", offset));
                    }
                    append_line(path, &line)?;
                }

                Command::Createlink { name, from, ops } => {
                    let mut block = format!("createlink {} from={}\n", name, from);
                    for op in ops {
                        block.push_str(&format!("  {}\n", op));
                    }
                    append_line(path, &block)?;
                }

                Command::Movepoint { point, dx, dy, to } => {
                    let parts: Vec<&str> = point.splitn(2, '.').collect();
                    if parts.len() != 2 {
                        anyhow::bail!(
                            "'{}' isn't a qualified point — use shape.point format\n\nExamples:\n  strok movepoint stem.base --to 200,300\n  strok movepoint badge.top --dx 0 --dy -10",
                            point
                        );
                    }
                    let shape_name = parts[0];
                    let point_name = parts[1];
                    let op_line = if let Some(to) = to {
                        format!("  movepoint {} to={}", point_name, to)
                    } else {
                        format!(
                            "  movepoint {} dx={} dy={}",
                            point_name,
                            dx.unwrap_or(0.0),
                            dy.unwrap_or(0.0)
                        )
                    };
                    insert_into_shape_block(path, shape_name, &op_line)?;
                }

                Command::Inspect {
                    selector,
                    svg,
                    detail,
                    json,
                    scheme,
                } => {
                    let text = std::fs::read_to_string(path)
                        .with_context(|| format!("failed to read '{}'", file))?;

                    // Structural snapshot (E3.2): --detail and/or --json. --json
                    // alone defaults to a structural snapshot.
                    if detail.is_some() || *json {
                        let parsed = dsl_parse::parse_file_with_path(&text, path)?;
                        let scene = resolve::apply_scheme(&parsed, scheme.as_deref())?;
                        let level = match detail.as_deref() {
                            Some(d) => strok_core::query::Detail::parse(d).ok_or_else(|| {
                                anyhow::anyhow!(
                                    "invalid --detail '{}' (use full|structural|summary)",
                                    d
                                )
                            })?,
                            None => strok_core::query::Detail::Structural,
                        };
                        let snap = strok_core::query::snapshot(&scene, level);
                        if *json {
                            print!("{}", snap.to_json().to_string_pretty());
                        } else if level == strok_core::query::Detail::Full {
                            // Full + text → just the resolved SVG.
                            if let Some(svg_out) = snap.svg {
                                print!("{}", svg_out);
                            }
                        } else {
                            // Human-readable structural / summary listing.
                            println!(
                                "document {}x{} — {} element(s)",
                                strok_core::types::fmt_num(snap.width),
                                strok_core::types::fmt_num(snap.height),
                                snap.elements.len()
                            );
                            for el in &snap.elements {
                                match (level, el.bbox) {
                                    (strok_core::query::Detail::Summary, _) => {
                                        println!("  {} ({})", el.name, el.kind);
                                    }
                                    (_, Some((x0, y0, x1, y1))) => {
                                        println!(
                                            "  {} ({}) bbox={},{} {}x{}",
                                            el.name,
                                            el.kind,
                                            strok_core::types::fmt_num(x0),
                                            strok_core::types::fmt_num(y0),
                                            strok_core::types::fmt_num(x1 - x0),
                                            strok_core::types::fmt_num(y1 - y0),
                                        );
                                    }
                                    (_, None) => println!("  {} ({})", el.name, el.kind),
                                }
                            }
                        }
                        return Ok(());
                    }

                    if *svg {
                        let parsed = dsl_parse::parse_file_with_path(&text, path)?;
                        let scene = resolve::apply_scheme(&parsed, scheme.as_deref())?;
                        if let Some(sel) = selector {
                            if scene.find_node(sel).is_some() {
                                let svg_out = resolve::resolve_scene_single_node(&scene, sel);
                                print!("{}", svg_out);
                            } else if let Some((preview_scene, preview_name)) =
                                scene.make_shape_preview(sel)
                            {
                                let svg_out = resolve::resolve_scene_single_node(
                                    &preview_scene,
                                    &preview_name,
                                );
                                print!("{}", svg_out);
                            } else {
                                anyhow::bail!("'{}' not found", sel);
                            }
                        } else {
                            let svg_out = resolve::resolve_scene(&scene);
                            print!("{}", svg_out);
                        }
                    } else {
                        if let Some(sel) = selector {
                            // Extract just the relevant block from DSL text
                            let scene = dsl_parse::parse_file_with_path(&text, path)?;
                            // Check if it's a shape
                            if let Some(shape) = scene.find_shape(sel) {
                                let emitted = dsl_emit::emit_scene(&scene);
                                // Find the shape block
                                let marker = format!("shape {} ", sel);
                                let link_marker = format!("createlink {} ", sel);
                                for line in emitted.lines() {
                                    if line.starts_with(&marker)
                                        || line.starts_with(&link_marker)
                                        || (line.starts_with("  ")
                                            && !line.starts_with("shape ")
                                            && !line.starts_with("place ")
                                            && !line.starts_with("createlink ")
                                            && !line.starts_with("group ")
                                            && !line.starts_with("documentsize"))
                                    {
                                        // Simple extraction — print indented lines following the marker
                                    }
                                }
                                // Better approach: emit just that shape
                                let single_scene = strok_core::scene::Scene {
                                    document_size: scene.document_size,
                                    imports: vec![],
                                    palette: Default::default(),
                                    design_tokens: vec![],
                                    lets: vec![],
                                    defaults: vec![],
                                    shapes: vec![shape.clone()],
                                    components: vec![],
                                    nodes: vec![],
                                    imported_shape_names: Default::default(),
                                };
                                let out = dsl_emit::emit_scene(&single_scene);
                                // Strip documentsize line
                                let relevant: Vec<&str> = out.lines().skip(2).collect();
                                println!("{}", relevant.join("\n"));
                            } else {
                                // Check if it's a place
                                let emitted = dsl_emit::emit_scene(&scene);
                                let place_marker = format!("place {} ", sel);
                                let mut found = false;
                                for line in emitted.lines() {
                                    if line.trim_start().starts_with(&place_marker) {
                                        println!("{}", line);
                                        found = true;
                                    }
                                }
                                if !found {
                                    anyhow::bail!("'{}' not found", sel);
                                }
                            }
                        } else {
                            // Print the whole file as-is
                            print!("{}", text);
                        }
                    }
                }

                Command::Audit { json } => {
                    let text = std::fs::read_to_string(path)
                        .with_context(|| format!("failed to read '{}'", file))?;
                    let scene = dsl_parse::parse_file_with_path(&text, path)?;
                    let findings = audit::audit(&scene);

                    if *json {
                        // `audit --json` (deferred from C6 → E5.3). Stable schema
                        // via the shared json builder, like every other --json.
                        print!("{}", audit::findings_to_json(&findings).to_string_pretty());
                    } else if findings.is_empty() {
                        eprintln!("no suggestions");
                    } else {
                        let total_savings: usize = findings.iter().map(|f| f.line_savings).sum();
                        for f in &findings {
                            let label = match f.kind {
                                audit::FindingKind::NearMirror => "mirror",
                                audit::FindingKind::UnusedComposition => "unused",
                                audit::FindingKind::RoughCatmull => "rough",
                                audit::FindingKind::IsolatedCatmull => "isolated-curve",
                                audit::FindingKind::RepeatedPlaceRhythm => "repeat",
                                audit::FindingKind::NearDuplicateGroups => "dup-group",
                                audit::FindingKind::MagicNumberRhythm => "magic-num",
                                audit::FindingKind::UnanchoredAdjacency => "anchor",
                                audit::FindingKind::TextCollision => "text-collision",
                                audit::FindingKind::UnanchoredLabel => "label-anchor",
                            };
                            eprintln!("  {}: {}", label, f.message);
                            if !f.detail.is_empty() {
                                eprintln!("          → {}", f.detail);
                            }
                            if !f.suggestion.is_empty() {
                                eprintln!("          fix: {}", f.suggestion);
                            }
                            eprintln!();
                        }
                        let suggestion_word = if findings.len() == 1 {
                            "suggestion"
                        } else {
                            "suggestions"
                        };
                        if total_savings > 0 {
                            eprintln!(
                                "  {} {}, ~{} lines reducible",
                                findings.len(),
                                suggestion_word,
                                total_savings,
                            );
                        } else {
                            eprintln!("  {} {}", findings.len(), suggestion_word);
                        }
                    }
                }

                Command::Render {
                    out,
                    width,
                    height,
                    bg,
                    color,
                    node,
                    region,
                    annotate,
                    outline,
                    scheme,
                } => {
                    let doc = Document::load(path)
                        .with_context(|| format!("failed to load '{}'", file))?;

                    let region = region.as_deref().map(parse_region_spec).transpose()?;
                    let outline = parse_outline_selection(outline)?;
                    let opts = RenderOptions {
                        width: *width,
                        height: *height,
                        background: bg.clone(),
                        color: color.clone(),
                        region,
                    };

                    match doc.scene.as_ref() {
                        // v2 / arena documents: no colorschemes.
                        None => {
                            if *annotate {
                                anyhow::bail!(
                                    "--annotate requires a v3 scene document (no scene found)"
                                );
                            }
                            if outline.is_some() {
                                anyhow::bail!(
                                    "--outline requires a v3 scene document (no scene found)"
                                );
                            }
                            let png = render_to_png(&doc, &opts)?;
                            match out {
                                Some(p) => {
                                    std::fs::write(p, &png)?;
                                    eprintln!("rendered to {}", p);
                                }
                                None => std::io::stdout().write_all(&png)?,
                            }
                        }
                        Some(scene) => {
                            // No --scheme + an --out + schemes defined → render base + all schemes.
                            let render_all = out.is_some()
                                && scheme.is_none()
                                && !scene.palette.schemes.is_empty();

                            if render_all {
                                let out_path = out.as_ref().unwrap();
                                let base = render_one(
                                    scene,
                                    None,
                                    node.as_deref(),
                                    &opts,
                                    *annotate,
                                    outline.as_ref(),
                                )?;
                                std::fs::write(out_path, &base)?;
                                eprintln!("rendered to {}", out_path);
                                for sc in &scene.palette.schemes {
                                    let png = render_one(
                                        scene,
                                        Some(&sc.name),
                                        node.as_deref(),
                                        &opts,
                                        *annotate,
                                        outline.as_ref(),
                                    )?;
                                    let p = suffixed(out_path, &sc.name);
                                    std::fs::write(&p, &png)?;
                                    eprintln!("rendered to {}", p);
                                }
                            } else {
                                let png = render_one(
                                    scene,
                                    scheme.as_deref(),
                                    node.as_deref(),
                                    &opts,
                                    *annotate,
                                    outline.as_ref(),
                                )?;
                                match out {
                                    Some(p) => {
                                        std::fs::write(p, &png)?;
                                        eprintln!("rendered to {}", p);
                                    }
                                    None => std::io::stdout().write_all(&png)?,
                                }
                            }
                        }
                    }
                }

                Command::Bool { op, ids, out } => {
                    bool_op(path, op, ids, out.as_deref())?;
                }

                Command::OutlineStroke { id, out } => {
                    outline_stroke_op(path, id, out.as_deref())?;
                }

                Command::Offset { id, delta, out } => {
                    offset_op(path, id, *delta, out.as_deref())?;
                }

                Command::Transform {
                    name,
                    rotate,
                    scale,
                    skew,
                    flip,
                } => {
                    transform_place(
                        path,
                        name,
                        *rotate,
                        *scale,
                        skew.as_deref(),
                        flip.as_deref(),
                    )?;
                }

                Command::ConvertPoint { point, to } => {
                    let parts: Vec<&str> = point.splitn(2, '.').collect();
                    if parts.len() != 2 {
                        anyhow::bail!(
                            "'{}' isn't a qualified point — use shape.point format\n\nExample:\n  strok convert-point leaf.tip --to arc",
                            point
                        );
                    }
                    if strok_core::shape::ConvertTarget::parse(to).is_none() {
                        anyhow::bail!(
                            "'{}' is not a valid target — use sharp, smooth, arc, or controls",
                            to
                        );
                    }
                    let op_line = format!("  convert-point {} to={}", parts[1], to);
                    insert_into_shape_block(path, parts[0], &op_line)?;
                }

                Command::TextOnPath {
                    text,
                    path: path_id,
                    name,
                    size,
                    fill,
                } => {
                    text_on_path(path, text, path_id, name.as_deref(), *size, fill.as_deref())?;
                }

                Command::Measure { a, b, json } => {
                    let text = std::fs::read_to_string(path)
                        .with_context(|| format!("failed to read '{}'", file))?;
                    let scene = dsl_parse::parse_file_with_path(&text, path)?;
                    let report = strok_core::measure::measure(&scene, a, b)
                        .map_err(|e| anyhow::anyhow!(e))?;
                    if *json {
                        print!("{}", report.to_json());
                    } else {
                        print!("{}", report.to_text());
                    }
                }

                Command::Query {
                    r#box,
                    overlaps,
                    json,
                } => {
                    let text = std::fs::read_to_string(path)
                        .with_context(|| format!("failed to read '{}'", file))?;
                    let scene = dsl_parse::parse_file_with_path(&text, path)?;
                    let result = match (r#box.as_deref(), overlaps.as_deref()) {
                        (Some(_), Some(_)) => {
                            anyhow::bail!("use either --box or --overlaps, not both");
                        }
                        (Some(spec), None) => {
                            let (x, y, w, h) = parse_box_spec(spec)?;
                            strok_core::query::query_box(&scene, x, y, w, h)
                        }
                        (None, Some(id)) => strok_core::query::query_overlaps(&scene, id)
                            .map_err(|e| anyhow::anyhow!(e))?,
                        (None, None) => {
                            anyhow::bail!("query requires --box x,y,w,h or --overlaps <id>");
                        }
                    };
                    if *json {
                        print!("{}", result.to_json().to_string_pretty());
                    } else {
                        println!("{} — {} match(es)", result.query, result.matches.len());
                        for el in &result.matches {
                            println!("  {} ({})", el.name, el.kind);
                        }
                    }
                }

                Command::Relate { a, b, json } => {
                    let text = std::fs::read_to_string(path)
                        .with_context(|| format!("failed to read '{}'", file))?;
                    let scene = dsl_parse::parse_file_with_path(&text, path)?;
                    let rel =
                        strok_core::query::relate(&scene, a, b).map_err(|e| anyhow::anyhow!(e))?;
                    if *json {
                        print!("{}", rel.to_json().to_string_pretty());
                    } else {
                        print!("{}", rel.to_text());
                    }
                }

                Command::Snap { name, mode, step } => {
                    snap_place(path, name, mode, *step)?;
                }

                Command::Reorder { name, position } => {
                    reorder_block(path, name, position)?;
                }

                Command::Export {
                    format,
                    out,
                    width,
                    height,
                    color,
                    scheme,
                } => {
                    let loaded = Document::load(path)
                        .with_context(|| format!("failed to load '{}'", file))?;
                    // Resolve palette tokens against the requested scheme (base if none).
                    let doc = match loaded.scene.as_ref() {
                        Some(s) => {
                            Document::from_scene(resolve::apply_scheme(s, scheme.as_deref())?)
                        }
                        None => loaded,
                    };

                    match format.as_str() {
                        "svg" => {
                            let svg_out = emit::emit_document(&doc);
                            match out {
                                Some(out_path) => std::fs::write(out_path, &svg_out)?,
                                None => print!("{}", svg_out),
                            }
                        }
                        "png" => {
                            let opts = RenderOptions {
                                width: *width,
                                height: *height,
                                background: None,
                                color: color.clone(),
                                region: None,
                            };
                            let png_bytes = render_to_png(&doc, &opts)?;
                            match out {
                                Some(out_path) => std::fs::write(out_path, &png_bytes)?,
                                None => std::io::stdout().write_all(&png_bytes)?,
                            }
                        }
                        _ => anyhow::bail!("unknown format '{}' (expected svg or png)", format),
                    }
                }

                Command::Emit {
                    target,
                    out,
                    name,
                    scheme,
                } => {
                    let text = std::fs::read_to_string(path)
                        .with_context(|| format!("failed to read '{}'", file))?;
                    let scene = dsl_parse::parse_file_with_path(&text, path)
                        .with_context(|| format!("failed to parse '{}'", file))?;

                    let emitter = strok_targets::target_by_id(target).ok_or_else(|| {
                        anyhow::anyhow!(
                            "unknown target '{}' (expected one of: {})",
                            target,
                            strok_targets::TARGET_IDS.join(", ")
                        )
                    })?;

                    let opts = strok_targets::EmitOptions {
                        component_name: name.clone(),
                        scheme: scheme.clone(),
                    };
                    let artifact = emitter
                        .emit(&scene, &opts)
                        .map_err(|e| anyhow::anyhow!("emit failed: {}", e))?;

                    // Diagnostics are surfaced on stderr — never silently dropped.
                    for d in &artifact.diagnostics {
                        eprintln!("note: {}", d);
                    }

                    match out {
                        Some(dir) => {
                            std::fs::create_dir_all(dir)?;
                            for f in &artifact.files {
                                let dest = Path::new(dir).join(&f.path);
                                std::fs::write(&dest, &f.contents)?;
                                eprintln!("wrote {}", dest.display());
                            }
                        }
                        None => {
                            for f in &artifact.files {
                                print!("{}", f.contents);
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

/// Render a scene under one colorscheme to PNG bytes, optionally a single node.
/// Parse a `x,y,w,h` region spec for `query --box`.
fn parse_box_spec(spec: &str) -> Result<(f64, f64, f64, f64)> {
    let parts: Vec<&str> = spec.split(',').collect();
    if parts.len() != 4 {
        anyhow::bail!(
            "--box expects x,y,w,h (four comma-separated numbers), got '{}'",
            spec
        );
    }
    let mut nums = [0.0f64; 4];
    for (i, p) in parts.iter().enumerate() {
        nums[i] = p
            .trim()
            .parse::<f64>()
            .map_err(|_| anyhow::anyhow!("--box value '{}' is not a number", p))?;
    }
    Ok((nums[0], nums[1], nums[2], nums[3]))
}

fn parse_region_spec(spec: &str) -> Result<RenderRegion> {
    let (x, y, width, height) = parse_box_spec(spec)
        .map_err(|_| anyhow::anyhow!("--region expects x,y,w,h, got '{}'", spec))?;
    Ok(RenderRegion {
        x,
        y,
        width,
        height,
    })
}

#[derive(Debug)]
enum OutlineSelection {
    All,
    Only(Vec<String>),
}

fn parse_outline_selection(arg: &Option<Option<String>>) -> Result<Option<OutlineSelection>> {
    match arg {
        None => Ok(None),
        Some(None) => Ok(Some(OutlineSelection::All)),
        Some(Some(spec)) => {
            let mut ids = Vec::new();
            for raw in spec.split(',') {
                let id = raw.trim();
                if id.is_empty() {
                    anyhow::bail!(
                        "--outline expects comma-separated placed IDs; omit the value to outline all"
                    );
                }
                if !ids.iter().any(|existing| existing == id) {
                    ids.push(id.to_string());
                }
            }
            if ids.is_empty() {
                anyhow::bail!(
                    "--outline expects comma-separated placed IDs; omit the value to outline all"
                );
            }
            Ok(Some(OutlineSelection::Only(ids)))
        }
    }
}

fn render_one(
    scene: &Scene,
    scheme: Option<&str>,
    node: Option<&str>,
    opts: &RenderOptions,
    annotate: bool,
    outline: Option<&OutlineSelection>,
) -> Result<Vec<u8>> {
    let resolved = resolve::apply_scheme(scene, scheme)?;
    let (dw, dh) = (resolved.document_size.w, resolved.document_size.h);

    let mut svg = match node {
        Some(n) => {
            if resolved.find_node(n).is_some() {
                resolve::resolve_scene_single_node(&resolved, n)
            } else if let Some((preview, preview_name)) = resolved.make_shape_preview(n) {
                resolve::resolve_scene_single_node(&preview, &preview_name)
            } else {
                eprintln!(
                    "warning: no placed element or shape named '{}' in scene — rendering empty canvas",
                    n
                );
                resolve::resolve_scene_single_node(&resolved, n)
            }
        }
        // Annotate mode (E3.2): overlay element IDs on the full canvas.
        None if annotate => resolve::resolve_scene_annotated(&resolved),
        None => resolve::resolve_scene(&resolved),
    };
    if let Some(selection) = outline {
        let ids = match selection {
            OutlineSelection::All => None,
            OutlineSelection::Only(ids) => Some(ids.as_slice()),
        };
        svg = resolve::add_outline_overlay(&svg, ids)?;
    }

    let (source_w, source_h) = opts
        .region
        .map(|region| (region.width, region.height))
        .unwrap_or((dw, dh));
    let (w, h) = target_dimensions(source_w, source_h, opts.width, opts.height);
    Ok(strok_render::render_svg_string(&svg, w, h, dw, dh, opts)?)
}

/// Insert a `-<suffix>` before the file extension: `icon.png` → `icon-dark.png`.
/// Build the DSL source for a new document (shared by the `new` CLI verb and the
/// MCP `new` tool). `size` is a `WxH` string; `profile` an optional preset. The
/// result is validated by parsing before being returned.
pub fn build_document_source(size: &str, profile: Option<&str>) -> Result<String> {
    let contents = match profile {
        None => format!("documentsize {}\n", size),
        Some("icon" | "icon-outline-round") => {
            // The icon profile defaults to a 24×24 grid; honor an explicit
            // non-default size (e.g. 16x16) if the caller set one.
            let size = if size == "800x800" { "24x24" } else { size };
            build_icon_profile(size, IconProfile::OutlineRound)
        }
        Some("icon-outline-angular") => {
            let size = if size == "800x800" { "24x24" } else { size };
            build_icon_profile(size, IconProfile::OutlineAngular)
        }
        Some("icon-solid") => {
            let size = if size == "800x800" { "24x24" } else { size };
            build_icon_profile(size, IconProfile::Solid)
        }
        Some("icon-mixed") => {
            let size = if size == "800x800" { "24x24" } else { size };
            build_icon_profile(size, IconProfile::Mixed)
        }
        Some(other) => anyhow::bail!(
            "unknown profile '{}' (known: icon-outline-round, icon-outline-angular, icon-solid, icon-mixed; 'icon' aliases icon-outline-round). Run `strok guide icon` before choosing",
            other
        ),
    };
    dsl_parse::parse_file(&contents).map_err(|e| anyhow::anyhow!("{}", e))?;
    Ok(contents)
}

/// Print diff stats as text or JSON, then return whether the pair is within the
/// golden perceptual tolerance (callers map this to the process exit status).
fn report_diff(stats: &strok_render::DiffStats, json: bool) -> bool {
    use strok_core::json::Json;
    if json {
        let bbox = match stats.changed_bbox {
            Some((x0, y0, x1, y1)) => Json::obj([
                ("x", Json::num(x0 as f64)),
                ("y", Json::num(y0 as f64)),
                ("w", Json::num((x1 - x0 + 1) as f64)),
                ("h", Json::num((y1 - y0 + 1) as f64)),
            ]),
            None => Json::Null,
        };
        let obj = Json::obj([
            (
                "mean_abs",
                Json::num((stats.mean_abs * 100.0).round() / 100.0),
            ),
            ("changed_pixels", Json::num(stats.changed_pixels as f64)),
            ("total_pixels", Json::num(stats.total_pixels as f64)),
            (
                "changed_fraction",
                Json::num((stats.changed_fraction * 10000.0).round() / 10000.0),
            ),
            ("changed_bbox", bbox),
            (
                "within_tolerance",
                Json::Bool(stats.within_golden_tolerance()),
            ),
        ]);
        print!("{}", obj.to_string_pretty());
    } else {
        eprintln!(
            "mean Δ {:.2}/255, {} of {} px changed ({:.2}%)",
            stats.mean_abs,
            stats.changed_pixels,
            stats.total_pixels,
            stats.changed_fraction * 100.0,
        );
        match stats.changed_bbox {
            Some((x0, y0, x1, y1)) => eprintln!(
                "changed region: {},{} {}x{}",
                x0,
                y0,
                x1 - x0 + 1,
                y1 - y0 + 1
            ),
            None => eprintln!("no changes"),
        }
    }
    stats.within_golden_tolerance()
}

/// `diff a.png b.png` — compare two PNG files (E3.3, reusing the golden comparator).
fn diff_images(a: &str, b: &str, out: Option<&str>, json: bool) -> Result<()> {
    let pa = std::fs::read(a).with_context(|| format!("failed to read '{}'", a))?;
    let pb = std::fs::read(b).with_context(|| format!("failed to read '{}'", b))?;
    let ia = strok_render::decode_png(&pa).map_err(|e| anyhow::anyhow!("{}: {}", a, e))?;
    let ib = strok_render::decode_png(&pb).map_err(|e| anyhow::anyhow!("{}: {}", b, e))?;
    let (stats, diff) = strok_render::compare(&ia, &ib).map_err(|e| anyhow::anyhow!("{}", e))?;
    if let Some(p) = out {
        let png = strok_render::encode_png(&diff).map_err(|e| anyhow::anyhow!("{}", e))?;
        std::fs::write(p, &png)?;
        eprintln!("wrote diff to {}", p);
    }
    let within = report_diff(&stats, json);
    if !within {
        std::process::exit(1);
    }
    Ok(())
}

/// `diff --since N` — render the document as of after op N and compare to the
/// current render (E3.3, replaying the op log via `Document::replay_to`).
#[allow(clippy::too_many_arguments)]
fn diff_since(
    path: &Path,
    a: Option<&str>,
    b: Option<&str>,
    since: Option<usize>,
    out: Option<&str>,
    width: Option<u32>,
    height: Option<u32>,
    color: Option<&str>,
    json: bool,
) -> Result<()> {
    // Two file args with --since is contradictory.
    if a.is_some() || b.is_some() {
        if since.is_some() {
            anyhow::bail!("provide either two image files OR --since, not both");
        }
        anyhow::bail!("diff needs two image files (a.png b.png) or --since <n>");
    }
    let since = since.ok_or_else(|| {
        anyhow::anyhow!("diff needs two image files (a.png b.png) or --since <n>")
    })?;

    let doc =
        Document::load(path).with_context(|| format!("failed to load '{}'", path.display()))?;

    if doc.history_len() <= 1 {
        anyhow::bail!(
            "no construction history to diff against — `--since` replays the op log, \
             which only binary-format .strok files persist. For DSL documents, render \
             two versions and use `strok diff before.png after.png`."
        );
    }
    if since >= doc.history_len() {
        anyhow::bail!(
            "--since {} is past the end of history ({} ops)",
            since,
            doc.history_len()
        );
    }

    let before_doc = doc
        .replay_to(since)
        .map_err(|e| anyhow::anyhow!("history replay failed: {}", e))?;

    let opts = RenderOptions {
        width,
        height,
        background: Some("#ffffff".into()),
        color: color.map(|s| s.to_string()),
        region: None,
    };
    let before_png = render_to_png(&before_doc, &opts)?;
    let after_png = render_to_png(&doc, &opts)?;

    let (stats, diff_png) = strok_render::diff_png_bytes(&before_png, &after_png)
        .map_err(|e| anyhow::anyhow!("{}", e))?;
    if let Some(p) = out {
        std::fs::write(p, &diff_png)?;
        eprintln!("wrote diff to {}", p);
    }
    let within = report_diff(&stats, json);
    if !within {
        std::process::exit(1);
    }
    Ok(())
}

#[derive(Clone, Copy)]
enum IconProfile {
    OutlineRound,
    OutlineAngular,
    Solid,
    Mixed,
}

/// Seed an icon document with an explicit visual grammar. The generated
/// comments are intentionally part of the agent interface: they keep the style
/// decision and review loop in context while geometry is being authored.
fn build_icon_profile(size: &str, profile: IconProfile) -> String {
    let (name, rationale, defaults, extra) = match profile {
        IconProfile::OutlineRound => (
            "round outline",
            "friendly/soft; use only when rounded terminals fit the product voice",
            "  fill none\n  stroke currentColor\n  stroke-width 2\n  stroke-linecap round\n  stroke-linejoin round\n",
            "",
        ),
        IconProfile::OutlineAngular => (
            "angular outline",
            "precise/technical; preserves decisive corners and flat terminals",
            "  fill none\n  stroke currentColor\n  stroke-width 2\n  stroke-linecap butt\n  stroke-linejoin miter\n  stroke-miterlimit 3\n",
            "",
        ),
        IconProfile::Solid => (
            "solid",
            "strongest silhouette and most reliable reading at very small sizes",
            "  fill currentColor\n  stroke none\n",
            "# Build the outer silhouette first; use negative space instead of hairline detail.\n",
        ),
        IconProfile::Mixed => (
            "mixed solid + line",
            "filled mass for recognition with sparse stroked detail for hierarchy",
            "  fill currentColor\n  stroke none\n",
            "# Opt only secondary detail shapes into: fill none; stroke currentColor; stroke-width 2.\n# Keep one dominant silhouette; do not outline every filled component.\n",
        ),
    };
    format!(
        "documentsize {size}\n\
         \n\
         # Icon profile: {name} — {rationale}.\n\
         # Live area: keep normal geometry inside 2,2 .. 22,22 on a 24-grid.\n\
         # Quality loop: render at the smallest shipping size AND at 4x; run `strok -f <file> audit`.\n\
         # Compare the whole set with `strok batch ... --sheet ...`; tune optical weight, not just coordinates.\n\
         {extra}\
         defaults\n\
         {defaults}"
    )
}

/// Agent-facing visual guidance shared by CLI and MCP.
pub fn guide_text(topic: &str) -> Result<&'static str> {
    match topic {
        "illustration" | "illustrations" => Ok(ILLUSTRATION_GUIDE),
        "icon" | "icons" => Ok(ICON_GUIDE),
        "logo" | "logos" => Ok(LOGO_GUIDE),
        "diagram" | "diagrams" => Ok(DIAGRAM_GUIDE),
        other => anyhow::bail!(
            "unknown guide topic '{}' (known: illustration, icon, logo, diagram)",
            other
        ),
    }
}

const AGENT_INTRO: &str = r#"AGENT INTRO — use Strøk as a visual construction and feedback system

Strøk source being valid is only the beginning. The deliverable is the rendered
image at its real viewing size. Plan enough visual review for the requested bar.

1. CHOOSE THE EFFORT LEVEL

   sketch       Establish composition and feasibility. Use 1–2 render/review
                passes. Appropriate for alternatives, wireframes, and discussion.

   production   Deliver a polished asset. Use at least 3–5 focused passes:
                composition, geometry, color/depth, detail, and final-size review.

   showcase     Demonstrate Strøk's expressive ceiling. Expect 6+ focused passes,
                full-frame AND region renders, deliberate materials and lighting,
                and a final cleanup pass at 2×. Words such as intricate, beautiful,
                editorial, hero, or complex imply this level unless scope says otherwise.

   Do not lower the effort because fewer commands are convenient. If time or tools
   cannot support the requested level, say what remains visually unverified.

2. DISCOVER BEFORE INVENTING

   strok --help                         complete DSL and primitive reference
   strok <command> --help               exact syntax and examples
   strok guide illustration             output-specific construction workflow
   strok lib list                       built-in modules and shape meanings
   strok lib search <meaning>           find reusable shapes by intent
   strok lib show <module>              read canonical Strøk source

3. BUILD LARGE TO SMALL

   Write a one-sentence visual brief that names the canvas, smallest delivery
   size, backgrounds, and schemes. Choose solid, mixed, angular-outline, or
   round-outline deliberately. Block the frame into foreground, subject, and
   background. Establish silhouettes and overlap before texture or decoration.
   Give each visual role a named shape: body, rim, inner rim, handle, cast shadow,
   highlight. Prefer primitives and a few intentional curves to dense tracing.
   A point's mode controls the segment ARRIVING at that point; at a smooth join,
   read the incoming handle → anchor → outgoing handle as one relationship.
   Reuse real geometry; do not force unrelated roles into one path.
   When several parts must read as one outer silhouette, put named placements in
   a live `boolean <name> op=union` block. It recomputes on every render while
   each operand stays available to `inspect` and `render --node`. Reserve the
   destructive `strok bool` command for an intentionally baked final path.

4. RUN THE VISUAL FEEDBACK LOOP

   strok -f art.strok render --out /tmp/full.png
   strok -f art.strok render --region x,y,w,h --width 1200 --out /tmp/detail.png
   strok -f art.strok render --outline body,rim,handle --region x,y,w,h --width 1200 --out /tmp/geometry.png
   strok -f art.strok render --annotate --out /tmp/names.png
   strok -f art.strok inspect --detail structural
   strok -f art.strok audit
   strok -f art.strok query --box x,y,w,h

   Inspect the actual images after every meaningful pass. For a complex scene,
   separately review every focal object and high-contrast edge. A thumbnail or
   contact sheet proves composition, not craft. Use `--outline` when silhouettes,
   joins, or attachment geometry are difficult to read through paint and shading.
   Change one concern at a time, render again, and use
   `strok diff before.png after.png` when the effect is subtle.
   Use one source writer at a time. Checkpoint before interactive point edits,
   then inspect the whole affected shape after each mutation.

5. FINISH WITH A VISUAL, NOT SYNTACTIC, GATE

   Check silhouette, joins, tangencies, clipping, z-order, perspective, material
   cues, repeated stroke/radius language, and details at delivery size. Run audit,
   but treat it as evidence rather than taste. Technically valid source is not a
   visual quality gate. The work is done only when the rendered result meets the
   requested effort level without explanation.
"#;

pub fn agent_intro_text() -> &'static str {
    AGENT_INTRO
}

const ILLUSTRATION_GUIDE: &str = r#"ILLUSTRATION WORKFLOW — build depth, material, and believable objects

1. Write the visual brief and choose sketch, production, or showcase effort
   (`strok agent-intro`). For showcase work, budget separate passes for:
   composition → silhouettes → depth/light → object construction → detail → cleanup.

2. Compose with large masses first. Use overlap, scale, value, and atmospheric
   contrast to establish foreground/midground/background. Render before adding
   small detail; a weak thumbnail cannot be rescued by more paths.

3. Construct objects from semantic parts. A cup is body + back handle + rim +
   liquid + inner rim + base shadow + highlight. An open book has a shared spine,
   curved page silhouettes, page thickness, and text lines that follow each page.
   Name those roles so `render --node`, `query`, and annotated renders stay useful.
   If those parts must share one seamless silhouette, combine their named places
   in a live `boolean ... op=union` block instead of copying or baking path data.

4. Match geometry to the material:
   - hard manufactured edges: arcs, true round-corners, or explicit Bézier controls
   - organic contours: consecutive Catmull-Rom runs, with intentional sharp tips
   - vessels and fabric: explicit cubic controls for tuned silhouettes
   - thin details: round caps, consistent width, and enough contrast to survive size
   Remember: a point mode controls the segment arriving at that point.

5. Create depth deliberately. Put cast shadows behind objects, contact shadows at
   their bases, occluded parts before foreground parts, and highlights last. Avoid
   a dark outline around every shape; edges can come from value and overlap.

6. Review focal objects independently:
     strok -f scene.strok render --region 450,340,220,180 --width 1200 --out /tmp/focal.png
     strok -f scene.strok render --annotate --out /tmp/names.png
     strok -f scene.strok audit
   At showcase level, inspect the full frame at delivery size and at least 2×,
   plus a high-resolution region for every focal object. Iterate on one visible
   problem at a time. A montage is not enough.

7. Illustration quality gates:
   - every attachment meets cleanly (handles, stems, limbs, leaves)
   - no accidental flat bottoms, cusps, gaps, or doubled edges
   - repeated perspective lines converge consistently
   - highlights and shadows agree on the light direction
   - details support the focal hierarchy instead of filling space
   - the image reads without a caption explaining malformed objects
"#;

const ICON_GUIDE: &str = r#"ICON WORKFLOW — optimize recognition and family coherence, not command count

1. Choose a visual grammar before drawing:
   icon-solid            strongest silhouette; best default for tiny/product marks
   icon-mixed            filled primary mass + sparse line detail; more hierarchy
   icon-outline-angular  precise/technical; butt caps + miter joins
   icon-outline-round    friendly/soft; round caps + joins (legacy `icon` alias)
   Do not choose round outline merely because it is familiar. Match the product.

2. Establish a set contract: grid, live area, optical weight, corner language,
   terminal language, fill/stroke policy, and smallest shipping size. Reuse it.

3. Build recognition from the silhouette. Prefer rectangle/ellipse/line, boolean
   operations, arcs, and true round-corners for geometric forms. Catmull-Rom is
   for organic runs. Point modes affect the incoming segment; use smooth-corner
   when the intent is explicitly both sides of one anchor. Use explicit controls
   for a logo-like or optically tuned curve.

4. Iterate visually: render at the smallest shipping size and at 4x, on light
   and dark backgrounds. Run audit after geometry changes. Review a contact sheet
   for optical size/weight drift. Simplify details that disappear at target size.

5. Quality gates: no accidental kinks, clipping, doubled shared edges, inconsistent
   radii, or one-off stroke widths. Pixel-snap axis-aligned even-width strokes.
   A coherent set may be outline-only; that should be a deliberate art direction.
"#;

const LOGO_GUIDE: &str = r#"LOGO WORKFLOW — optimize distinctiveness, silhouette, and reproduction

1. Start in one color. Choose a concept and 1–3 decisive shapes; avoid assembling
   a generic rounded-outline badge from the icon defaults. A logo is not an icon
   with more detail.

2. Build geometry from primitives, booleans, true corner fillets, and explicit
   Bézier controls. Use symmetry as construction help, then make optical corrections
   where mathematical centering looks wrong. Prefer negative space over thin seams.

3. If strokes define the mark, test cap/join/miter choices deliberately and use
   outline-stroke before final delivery when the silhouette must be invariant.

4. Review at favicon size, normal UI size, and large display size; test light/dark
   and pure monochrome. Remove anything that only works at the large size. Compare
   against adjacent-category marks for distinctiveness, not merely polish.

5. Keep construction editable in .strok, but judge the exported silhouette. Run
   audit, inspect the SVG, and iterate until curves, spacing, and counterforms hold
   at every required size. More render-review passes are preferable to a quick
   technically valid mark.
"#;

const DIAGRAM_GUIDE: &str = r#"DIAGRAM WORKFLOW — make relationships legible and keep labels attached

1. Name the one relationship the diagram must explain. Choose a directional
   sequence, comparison, hierarchy, or system map; do not decorate empty space.

2. Build geometry before labels. Reuse shapes and use repeat for real rhythms.
   Keep routes behind nodes and reserve one accent color for the decisive state.

3. Attach text by intent, not by guessed baselines:
     place node-label shape=label at=node.center align=center
     place node-note  shape=note  below=node gap=8
     place side-label shape=label at=node.left align=right offset=-8,0
   A plain text `at=x,y` is SVG baseline-start placement. Reserve it for free
   editorial text; it is font-sensitive and easy to misread as a top-left point.

4. Run `strok audit` before export. A `text-collision` finding means a label
   partially clips closed geometry; use its concrete relative-anchor rewrite,
   render, inspect, and iterate. Fully contained labels are treated as intentional.
   Use `query --overlaps <id>` when investigating a dense local region.

5. Review at the actual delivery size and at 2×. Check reading order, connector
   endpoints, optical centering, edge clearance, and font availability. Exported
   SVG text relies on the viewer having the named font, so prefer resilient font
   stacks or outline final text in a downstream tool when exact reproduction is
   mandatory.
"#;

/// Arguments to [`batch_render`]. A struct keeps the signature manageable as the
/// design-system outputs (sprite/sheet/manifest) accreted onto the icon pipeline.
struct BatchArgs<'a> {
    dir: &'a str,
    out: Option<&'a str>,
    svg: bool,
    png: bool,
    sizes: Option<&'a str>,
    color: Option<&'a str>,
    bg: Option<&'a str>,
    scheme: Option<&'a str>,
    sprite: Option<&'a str>,
    sheet: Option<&'a str>,
    manifest: Option<&'a str>,
    columns: Option<u32>,
}

/// Batch-render every `*.strok` in `dir` to SVG and/or PNG, plus the optional
/// design-system artifacts (sprite `<symbol>` sheet, contact sheet, manifest).
/// Built for icon sets: one clean themeable SVG per file plus PNGs at each size,
/// all driven off the same parse so the per-icon files, the sprite, the sheet,
/// and the manifest can never drift.
fn batch_render(args: BatchArgs) -> Result<()> {
    let BatchArgs {
        dir,
        out,
        svg,
        png,
        sizes,
        color,
        bg,
        scheme,
        sprite,
        sheet,
        manifest: manifest_path,
        columns,
    } = args;

    let dir_path = Path::new(dir);
    if !dir_path.is_dir() {
        anyhow::bail!("'{}': not a directory", dir);
    }
    // Default output dir: <dir>/dist.
    let out_dir = out
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| dir_path.join("dist"));
    std::fs::create_dir_all(&out_dir)
        .with_context(|| format!("failed to create output dir '{}'", out_dir.display()))?;

    // Aggregate outputs imply work even if neither --svg/--png was named.
    let want_aggregate = sprite.is_some() || sheet.is_some() || manifest_path.is_some();
    // Neither flag → both. Otherwise honor exactly what was asked.
    let (mut want_svg, mut want_png) = if !svg && !png {
        (true, true)
    } else {
        (svg, png)
    };
    // A sprite needs the SVG; a contact sheet needs a PNG render. Ensure the
    // dependency is produced even if the user only asked for the aggregate.
    if sprite.is_some() {
        want_svg = true;
    }
    if sheet.is_some() {
        want_png = true;
    }

    // Parse sizes (default a single 24 px).
    let sizes: Vec<u32> = match sizes {
        Some(s) => s
            .split(',')
            .map(|t| t.trim())
            .filter(|t| !t.is_empty())
            .map(|t| {
                t.parse::<u32>()
                    .map_err(|_| anyhow::anyhow!("invalid size '{}' in --sizes", t))
            })
            .collect::<Result<Vec<_>>>()?,
        None => vec![24],
    };
    if want_png && sizes.is_empty() {
        anyhow::bail!("--sizes resolved to no values");
    }
    let single_size = sizes.len() == 1;
    // The contact sheet uses the largest requested size so tiles are crisp.
    let sheet_size = sizes.iter().copied().max().unwrap_or(24);

    // Collect *.strok files, sorted for deterministic output.
    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir_path)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("strok"))
        .collect();
    files.sort();
    if files.is_empty() {
        anyhow::bail!("no .strok files found in '{}'", dir);
    }

    let mut count = 0usize;
    let mut sprite_symbols: Vec<SpriteSymbol> = Vec::new();
    let mut sheet_tiles: Vec<SheetTile> = Vec::new();
    let mut manifest_entries: Vec<IconEntry> = Vec::new();
    let mut visual_grammars: BTreeMap<&'static str, usize> = BTreeMap::new();

    for file in &files {
        let stem = file
            .file_stem()
            .and_then(|s| s.to_str())
            .unwrap_or("icon")
            .to_string();
        let source = std::fs::read_to_string(file)
            .with_context(|| format!("failed to read '{}'", file.display()))?;
        let loaded =
            Document::load(file).with_context(|| format!("failed to load '{}'", file.display()))?;

        if let Some(scene) = loaded.scene.as_ref() {
            *visual_grammars
                .entry(classify_visual_grammar(scene))
                .or_default() += 1;
        }

        // Resolve palette tokens against the requested scheme (base if none),
        // exactly like `export`/`render`.
        let doc = match loaded.scene.as_ref() {
            Some(s) => Document::from_scene(resolve::apply_scheme(s, scheme)?),
            None => loaded,
        };

        let svg_out = emit::emit_document(&doc);

        if want_svg {
            let p = out_dir.join(format!("{}.svg", stem));
            std::fs::write(&p, &svg_out)?;
        }
        if want_png {
            for &size in &sizes {
                let opts = RenderOptions {
                    width: Some(size),
                    height: Some(size),
                    background: bg.map(|s| s.to_string()),
                    color: color.map(|s| s.to_string()),
                    region: None,
                };
                let png_bytes = render_to_png(&doc, &opts)?;
                let name = if single_size {
                    format!("{}.png", stem)
                } else {
                    format!("{}-{}.png", stem, size)
                };
                std::fs::write(out_dir.join(name), &png_bytes)?;
            }
        }

        // Sprite symbol: reuse the SVG we just emitted (currentColor preserved).
        if sprite.is_some() {
            if let Some((viewbox, inner)) = manifest::split_svg(&svg_out) {
                sprite_symbols.push(SpriteSymbol {
                    id: stem.clone(),
                    viewbox,
                    inner,
                });
            }
        }
        // Contact-sheet tile: render once at the sheet size (background applied
        // by the sheet itself, so render transparent for clean compositing).
        if sheet.is_some() {
            let opts = RenderOptions {
                width: Some(sheet_size),
                height: Some(sheet_size),
                background: None,
                color: color.map(|s| s.to_string()),
                region: None,
            };
            sheet_tiles.push(SheetTile {
                name: stem.clone(),
                png: render_to_png(&doc, &opts)?,
            });
        }
        // Manifest entry: meaning/tags from the header comments + canvas + sizes.
        if manifest_path.is_some() {
            let meta = manifest::parse_meta(&source);
            let canvas = doc
                .scene
                .as_ref()
                .map(|s| (s.document_size.w, s.document_size.h))
                .unwrap_or((doc.width, doc.height));
            manifest_entries.push(IconEntry {
                name: stem.clone(),
                meaning: meta.meaning,
                tags: meta.tags,
                canvas,
                sizes: sizes.clone(),
            });
        }

        count += 1;
    }

    // Write the aggregate artifacts.
    if let Some(sprite_path) = sprite {
        let sheet_svg = manifest::build_sprite(&sprite_symbols);
        write_aggregate(sprite_path, sheet_svg.as_bytes())?;
    }
    if let Some(sheet_path) = sheet {
        let opts = SheetOptions {
            columns: columns.unwrap_or(8),
            padding: 8,
            background: bg.map(|s| s.to_string()),
        };
        let png = contact_sheet(&sheet_tiles, &opts)
            .map_err(|e| anyhow::anyhow!("contact sheet: {}", e))?;
        write_aggregate(sheet_path, &png)?;
    }
    if let Some(mpath) = manifest_path {
        let m = Manifest {
            version: manifest::MANIFEST_VERSION,
            icons: manifest_entries,
        };
        write_aggregate(mpath, m.to_json_string().as_bytes())?;
    }

    let mut kinds = Vec::new();
    if want_svg {
        kinds.push("svg".to_string());
    }
    if want_png {
        kinds.push(format!(
            "png@{}",
            sizes
                .iter()
                .map(|s| s.to_string())
                .collect::<Vec<_>>()
                .join("/")
        ));
    }
    if sprite.is_some() {
        kinds.push("sprite".to_string());
    }
    if sheet.is_some() {
        kinds.push("sheet".to_string());
    }
    if manifest_path.is_some() {
        kinds.push("manifest".to_string());
    }
    let _ = want_aggregate;
    eprintln!(
        "batch: rendered {} file(s) → {} ({})",
        count,
        out_dir.display(),
        kinds.join(", ")
    );
    let grammar_summary = visual_grammars
        .iter()
        .map(|(name, count)| format!("{}={}", name, count))
        .collect::<Vec<_>>()
        .join(", ");
    eprintln!(
        "batch: visual grammar → {}. Consistency is useful; confirm the style is intentional (`strok guide icon`).",
        grammar_summary
    );
    Ok(())
}

/// Coarse, set-review classification based on the document's effective default
/// paint. This is intentionally descriptive rather than an audit failure: a
/// round-outline family can be excellent, but the corpus showed agents choosing
/// it reflexively because it was the only profile. Surfacing the distribution at
/// batch time makes that art-direction choice visible.
fn classify_visual_grammar(scene: &Scene) -> &'static str {
    use strok_core::shape::Operation;
    use strok_core::types::{Color, LineCap, LineJoin};

    let default_fill = scene.defaults.iter().rev().find_map(|op| match op {
        Operation::Fill(c) => Some(c),
        _ => None,
    });
    let default_stroke = scene.defaults.iter().rev().find_map(|op| match op {
        Operation::Stroke(c) => Some(c),
        _ => None,
    });
    let has_paint = |color: Option<&Color>| matches!(color, Some(c) if !matches!(c, Color::None));

    let default_has_fill = has_paint(default_fill);
    let default_has_stroke = has_paint(default_stroke);
    if !default_has_fill && default_has_stroke {
        let cap = scene.defaults.iter().rev().find_map(|op| match op {
            Operation::StrokeLinecap(c) => Some(*c),
            _ => None,
        });
        let join = scene.defaults.iter().rev().find_map(|op| match op {
            Operation::StrokeLinejoin(j) => Some(*j),
            _ => None,
        });
        return match (cap, join) {
            (Some(LineCap::Round), Some(LineJoin::Round)) => "outline-round",
            (Some(LineCap::Butt | LineCap::Square), Some(LineJoin::Miter | LineJoin::Bevel)) => {
                "outline-angular"
            }
            _ => "outline",
        };
    }

    if default_has_fill && !default_has_stroke {
        let has_stroked_detail = scene.shapes.iter().any(|shape| {
            has_paint(shape.stroke()) && matches!(shape.fill(), Some(Color::None) | None)
        });
        return if has_stroked_detail { "mixed" } else { "solid" };
    }

    let filled = scene
        .shapes
        .iter()
        .filter(|shape| has_paint(shape.fill()))
        .count();
    let stroked = scene
        .shapes
        .iter()
        .filter(|shape| has_paint(shape.stroke()))
        .count();
    match (filled > 0, stroked > 0) {
        (true, true) => "mixed",
        (true, false) => "solid",
        (false, true) => {
            let stroked_shapes = scene
                .shapes
                .iter()
                .filter(|shape| has_paint(shape.stroke()))
                .collect::<Vec<_>>();
            if stroked_shapes.iter().all(|shape| {
                shape.stroke_linecap() == Some(LineCap::Round)
                    && shape.stroke_linejoin() == Some(LineJoin::Round)
            }) {
                "outline-round"
            } else if stroked_shapes.iter().all(|shape| {
                matches!(
                    shape.stroke_linecap(),
                    Some(LineCap::Butt | LineCap::Square)
                ) && matches!(
                    shape.stroke_linejoin(),
                    Some(LineJoin::Miter | LineJoin::Bevel)
                )
            }) {
                "outline-angular"
            } else {
                "outline"
            }
        }
        (false, false) => "unstyled",
    }
}

/// Write an aggregate artifact (sprite/sheet/manifest) to an arbitrary path,
/// creating its parent directory if needed.
fn write_aggregate(path: &str, bytes: &[u8]) -> Result<()> {
    let p = Path::new(path);
    if let Some(parent) = p.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent)
                .with_context(|| format!("failed to create dir for '{}'", path))?;
        }
    }
    std::fs::write(p, bytes).with_context(|| format!("failed to write '{}'", path))?;
    Ok(())
}

/// Token sync (E5.3): cross-check an icon set's `$token` references against a
/// design-system file's defined tokens. Read-only; exits non-zero when an icon
/// references an undefined token.
fn token_sync_cmd(dir: &str, system: &str, json: bool) -> Result<()> {
    let dir_path = Path::new(dir);
    if !dir_path.is_dir() {
        anyhow::bail!("'{}': not a directory", dir);
    }
    let system_src = std::fs::read_to_string(system)
        .with_context(|| format!("failed to read design system '{}'", system))?;
    let system_scene = dsl_parse::parse_file(&system_src)
        .with_context(|| format!("failed to parse design system '{}'", system))?;

    let mut files: Vec<std::path::PathBuf> = std::fs::read_dir(dir_path)?
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("strok"))
        .collect();
    files.sort();

    let mut refs = std::collections::BTreeSet::new();
    for file in &files {
        let src = std::fs::read_to_string(file)
            .with_context(|| format!("failed to read '{}'", file.display()))?;
        refs.extend(token_sync::references_in_source(&src));
    }

    let report = token_sync::sync(&refs, &system_scene);

    if json {
        print!("{}", report.to_json_string());
    } else if report.is_in_sync() {
        eprintln!(
            "token-sync: in sync — {} reference(s) resolve against {} defined token(s)",
            report.matched.len(),
            report.defined.len(),
        );
        if !report.unused.is_empty() {
            eprintln!(
                "  unused (defined, no icon uses): {}",
                report.unused.join(", ")
            );
        }
    } else {
        eprintln!(
            "token-sync: OUT OF SYNC — {} undefined token reference(s):",
            report.undefined.len()
        );
        for u in &report.undefined {
            eprintln!(
                "  ${}  (referenced by an icon, not defined in {})",
                u, system
            );
        }
        if !report.unused.is_empty() {
            eprintln!(
                "  unused (defined, no icon uses): {}",
                report.unused.join(", ")
            );
        }
    }

    if !report.is_in_sync() {
        std::process::exit(1);
    }
    Ok(())
}

/// `strok import <in.svg> --out <out.strok>` (EXP-3): convert an SVG into an
/// editable .strok document with structure recovery. Stateless / no `-f` needed.
fn import_svg_cmd(input: &str, out: &str, json: bool) -> Result<()> {
    use strok_core::import_svg;

    let src = std::fs::read_to_string(input)
        .with_context(|| format!("failed to read SVG '{}'", input))?;
    let result = import_svg::import_svg(&src);

    // The emitted DSL is validated by re-parsing before it is written.
    let dsl = dsl_emit::emit_scene(&result.scene);
    dsl_parse::parse_file(&dsl)
        .with_context(|| "imported document failed to re-parse (importer bug)".to_string())?;
    std::fs::write(out, &dsl).with_context(|| format!("failed to write '{}'", out))?;

    if json {
        use strok_core::json::Json;
        let counts = Json::array(result.counts.iter().map(|(k, v)| {
            Json::obj([
                ("kind", Json::str(k.clone())),
                ("count", Json::num(*v as f64)),
            ])
        }));
        let tokens = Json::array(result.tokens.iter().map(|t| Json::str(t.clone())));
        let warnings = Json::array(result.warnings.iter().map(|w| {
            Json::obj([
                ("message", Json::str(w.message.clone())),
                (
                    "line",
                    match w.line {
                        Some(l) => Json::num(l as f64),
                        None => Json::Null,
                    },
                ),
            ])
        }));
        let obj = Json::obj([
            ("out", Json::str(out.to_string())),
            (
                "document",
                Json::obj([
                    ("w", Json::num(result.scene.document_size.w)),
                    ("h", Json::num(result.scene.document_size.h)),
                ]),
            ),
            ("shapes", Json::num(result.scene.shapes.len() as f64)),
            ("elements", counts),
            ("tokens", tokens),
            ("warnings", warnings),
        ]);
        print!("{}", obj.to_string_pretty());
    } else {
        let total: usize = result.counts.iter().map(|(_, v)| v).sum();
        eprintln!(
            "imported {} → {} ({} element(s), {} shape(s), {} token(s))",
            input,
            out,
            total,
            result.scene.shapes.len(),
            result.tokens.len(),
        );
        for w in &result.warnings {
            match w.line {
                Some(l) => eprintln!("  warning (line {}): {}", l, w.message),
                None => eprintln!("  warning: {}", w.message),
            }
        }
    }
    Ok(())
}

/// `strok lib list|show|search` (EXP-1): discoverability for the embedded
/// standard shape library — no `-f <file>` needed, this is a static registry
/// baked into the binary.
fn lib_cmd(action: &LibAction) -> Result<()> {
    match action {
        LibAction::List { json } => {
            let modules: Vec<(&'static str, Vec<stdlib::ShapeMeta>)> = stdlib::modules()
                .iter()
                .map(|m| (m.name, stdlib::shapes_meta(m.source)))
                .collect();

            if *json {
                let j = Json::array(modules.iter().map(|(name, shapes)| {
                    Json::obj([
                        ("module", Json::str(*name)),
                        (
                            "shapes",
                            Json::array(shapes.iter().map(|s| {
                                Json::obj([
                                    ("name", Json::str(&s.name)),
                                    ("meaning", Json::str(&s.meaning)),
                                    ("tags", Json::array(s.tags.iter().map(Json::str))),
                                ])
                            })),
                        ),
                    ])
                }));
                print!("{}", j.to_string_pretty());
            } else {
                for (name, shapes) in &modules {
                    println!("std/{}", name);
                    for s in shapes {
                        if s.meaning.is_empty() {
                            println!("  {}", s.name);
                        } else {
                            println!("  {:<20} {}", s.name, s.meaning);
                        }
                    }
                }
            }
        }

        LibAction::Show { module } => {
            let name = stdlib::strip_std_prefix(module).unwrap_or(module.as_str());
            let source = stdlib::get(name).ok_or_else(|| {
                anyhow::anyhow!(
                    "unknown standard library module '{}' — available modules: {}",
                    module,
                    stdlib::available_names()
                )
            })?;
            print!("{}", source);
        }

        LibAction::Search { query, json } => {
            let q = query.to_lowercase();
            let mut matches: Vec<(String, stdlib::ShapeMeta)> = Vec::new();
            for m in stdlib::modules() {
                for s in stdlib::shapes_meta(m.source) {
                    let hay = format!("{} {} {} {}", m.name, s.name, s.meaning, s.tags.join(" "))
                        .to_lowercase();
                    if hay.contains(&q) {
                        matches.push((m.name.to_string(), s));
                    }
                }
            }

            if *json {
                let j = Json::array(matches.iter().map(|(module, s)| {
                    Json::obj([
                        ("module", Json::str(module)),
                        ("name", Json::str(&s.name)),
                        ("meaning", Json::str(&s.meaning)),
                        ("tags", Json::array(s.tags.iter().map(Json::str))),
                    ])
                }));
                print!("{}", j.to_string_pretty());
            } else if matches.is_empty() {
                eprintln!("no matches for '{}'", query);
            } else {
                for (module, s) in &matches {
                    println!("std/{}.{:<20} {}", module, s.name, s.meaning);
                }
            }
        }
    }
    Ok(())
}

fn suffixed(path: &str, suffix: &str) -> String {
    let p = Path::new(path);
    let stem = p.file_stem().and_then(|s| s.to_str()).unwrap_or(path);
    let name = match p.extension().and_then(|e| e.to_str()) {
        Some(ext) => format!("{}-{}.{}", stem, suffix, ext),
        None => format!("{}-{}", stem, suffix),
    };
    match p.parent().filter(|d| !d.as_os_str().is_empty()) {
        Some(dir) => dir.join(name).to_string_lossy().into_owned(),
        None => name,
    }
}

/// Apply an affine transform to a placed element in place (E2.3).
///
/// Mutates the place's rotation / skew / flip / size through the scene model,
/// then re-emits the document. rotate/skew accumulate onto any existing values;
/// scale multiplies the current size; flip sets the axis. The whole document is
/// re-emitted from the parsed scene, so the result is guaranteed to round-trip.
fn transform_place(
    path: &Path,
    name: &str,
    rotate: Option<f64>,
    scale: Option<f64>,
    skew: Option<&str>,
    flip: Option<&str>,
) -> Result<()> {
    use strok_core::types::{Flip, Rotation};

    if rotate.is_none() && scale.is_none() && skew.is_none() && flip.is_none() {
        anyhow::bail!("transform needs at least one of --rotate, --scale, --skew, --flip");
    }

    let text = std::fs::read_to_string(path)?;
    let mut scene = dsl_parse::parse_file_with_path(&text, path)?;

    let place = find_place_mut(&mut scene.nodes, name)
        .ok_or_else(|| anyhow::anyhow!("no placed element named '{}'", name))?;

    if let Some(deg) = rotate {
        let cur = place.rotation.map(|r| r.0).unwrap_or(0.0);
        place.rotation = Some(Rotation(cur + deg));
    }
    if let Some(factor) = scale {
        if let Some(dim) = place.size.as_mut() {
            dim.w *= factor;
            dim.h *= factor;
        } else {
            anyhow::bail!(
                "'{}' has no explicit size= to scale — set a size first (place … size=WxH)",
                name
            );
        }
    }
    if let Some(sk) = skew {
        let (sx, sy) = parse_skew_arg(sk)?;
        let (cx, cy) = place.skew.unwrap_or((0.0, 0.0));
        place.skew = Some((cx + sx, cy + sy));
    }
    if let Some(fl) = flip {
        place.flip = Some(match fl {
            "x" => Flip::X,
            "y" => Flip::Y,
            "xy" => Flip::XY,
            _ => anyhow::bail!("invalid flip '{}' (expected x, y, or xy)", fl),
        });
    }

    let out = strok_core::dsl_emit::emit_scene(&scene);
    dsl_parse::parse_file_with_path(&out, path)
        .with_context(|| "re-emitted document failed to parse (transform bug)".to_string())?;
    std::fs::write(path, &out)?;
    eprintln!("transform {}: applied", name);
    Ok(())
}

/// Parse a `--skew` argument: `deg` or `degx,degy`.
fn parse_skew_arg(s: &str) -> Result<(f64, f64)> {
    let parts: Vec<&str> = s.split(',').collect();
    let p = |x: &str| -> Result<f64> {
        x.trim()
            .trim_end_matches("deg")
            .parse::<f64>()
            .map_err(|_| anyhow::anyhow!("invalid skew value '{}'", s))
    };
    match parts.len() {
        1 => Ok((p(parts[0])?, 0.0)),
        2 => Ok((p(parts[0])?, p(parts[1])?)),
        _ => anyhow::bail!("skew takes deg or degx,degy"),
    }
}

/// Find a place by name anywhere in the node tree (incl. inside groups).
fn find_place_mut<'a>(
    nodes: &'a mut [strok_core::scene::SceneNode],
    name: &str,
) -> Option<&'a mut strok_core::scene::Place> {
    use strok_core::scene::SceneNode;
    for node in nodes {
        match node {
            SceneNode::Place(p) if p.name == name => return Some(p),
            SceneNode::Group(g) => {
                if let Some(found) = find_place_mut(&mut g.children, name) {
                    return Some(found);
                }
            }
            _ => {}
        }
    }
    None
}

/// Flow a text string along a placed path (E2.7). Appends a `text` shape + a
/// `place … textpath=<path>` block; validated before writing.
fn text_on_path(
    path: &Path,
    text: &str,
    path_id: &str,
    name: Option<&str>,
    size: Option<f64>,
    fill: Option<&str>,
) -> Result<()> {
    let doc_text = std::fs::read_to_string(path)?;
    let scene = dsl_parse::parse_file_with_path(&doc_text, path)?;
    // The target must be a placed element with fillable geometry.
    if resolve::placed_document_d(&scene, path_id).is_none() {
        anyhow::bail!(
            "no placed path named '{}' to flow text along (--path takes a PLACE name)",
            path_id
        );
    }

    let place_name = name.unwrap_or("text-on-path");
    let shape_name = format!("{}-text", place_name);
    let fill = fill.unwrap_or("currentColor");

    let mut block = format!("shape {} template=text\n", shape_name);
    block.push_str(&format!("  content \"{}\"\n", text.replace('"', "\\\"")));
    if let Some(s) = size {
        block.push_str(&format!("  font-size {}\n", s));
    }
    block.push_str(&format!("  fill {}\n", fill));
    block.push_str(&format!(
        "place {} shape={} at=0,0 textpath={}\n",
        place_name, shape_name, path_id
    ));

    append_line(path, &block)?;
    eprintln!("text-on-path {}: along {}", place_name, path_id);
    Ok(())
}

/// Snap a placed element's `at=` position to a grid / edge / center (E2.7).
fn snap_place(path: &Path, name: &str, mode: &str, step: Option<f64>) -> Result<()> {
    use strok_core::measure::{snap_point, SnapMode};
    use strok_core::scene::PlacePosition;

    let snap_mode = SnapMode::parse(mode).ok_or_else(|| {
        anyhow::anyhow!("invalid snap mode '{}' (expected grid, edge, center)", mode)
    })?;

    let text = std::fs::read_to_string(path)?;
    let mut scene = dsl_parse::parse_file_with_path(&text, path)?;
    let (w, h) = (scene.document_size.w, scene.document_size.h);

    let place = find_place_mut(&mut scene.nodes, name)
        .ok_or_else(|| anyhow::anyhow!("no placed element named '{}'", name))?;

    match &mut place.position {
        PlacePosition::At(x, y) => {
            let (nx, ny) = snap_point((*x, *y), snap_mode, step.unwrap_or(8.0), w, h);
            *x = nx;
            *y = ny;
        }
        _ => {
            eprintln!(
                "note: '{}' uses relative/parametric placement — nothing to snap",
                name
            );
            return Ok(());
        }
    }

    let out = strok_core::dsl_emit::emit_scene(&scene);
    dsl_parse::parse_file_with_path(&out, path)
        .with_context(|| "re-emitted document failed to parse (snap bug)".to_string())?;
    std::fs::write(path, &out)?;
    eprintln!("snap {}: {} applied", name, mode);
    Ok(())
}

/// Boolean-combine placed shapes into a new path shape (E2.1).
fn bool_op(path: &Path, op: &str, ids: &[String], out: Option<&str>) -> Result<()> {
    use strok_core::bool_ops::{self, BoolOp};

    let bop = BoolOp::parse(op).map_err(|e| anyhow::anyhow!("{}", e))?;
    let text = std::fs::read_to_string(path)?;
    let scene = dsl_parse::parse_file_with_path(&text, path)?;

    let mut operands = Vec::with_capacity(ids.len());
    for id in ids {
        let d = resolve::placed_document_d(&scene, id).ok_or_else(|| {
            anyhow::anyhow!(
                "no placed element named '{}' with fillable geometry (boolean inputs are PLACE names)",
                id
            )
        })?;
        let rule = resolve::placed_fill_rule(&scene, id);
        operands.push((d, rule));
    }

    let out_name = out
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}-result", bop.name()));
    let shape = bool_ops::combine(bop, &operands, &out_name);
    append_generated_shape(path, &shape, &out_name)?;
    eprintln!(
        "bool {}: {} → shape/place '{}'",
        bop.name(),
        ids.join(", "),
        out_name
    );
    Ok(())
}

/// `outline-stroke` (E2.2): stroke → filled path shape.
fn outline_stroke_op(path: &Path, id: &str, out: Option<&str>) -> Result<()> {
    use strok_core::stroke_outline;

    let text = std::fs::read_to_string(path)?;
    let scene = dsl_parse::parse_file_with_path(&text, path)?;
    let d = resolve::placed_document_d(&scene, id)
        .ok_or_else(|| anyhow::anyhow!("no placed element named '{}' with geometry", id))?;
    let style = resolve::placed_stroke_style(&scene, id).unwrap_or_default();
    if style.width <= 0.0 {
        anyhow::bail!("'{}' has no positive stroke-width — nothing to outline", id);
    }
    let result =
        stroke_outline::outline_stroke_d(&d, &style).map_err(|e| anyhow::anyhow!("{}", e))?;
    let out_name = out
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}-outline", id));
    let shape = stroke_outline::shapes_to_shape(&out_name, &result);
    append_generated_shape(path, &shape, &out_name)?;
    eprintln!("outline-stroke {}: → shape/place '{}'", id, out_name);
    Ok(())
}

/// `offset` (E2.2): grow/inset a placed shape into a new path shape.
fn offset_op(path: &Path, id: &str, delta: f64, out: Option<&str>) -> Result<()> {
    use strok_core::stroke_outline;

    let text = std::fs::read_to_string(path)?;
    let scene = dsl_parse::parse_file_with_path(&text, path)?;
    let d = resolve::placed_document_d(&scene, id)
        .ok_or_else(|| anyhow::anyhow!("no placed element named '{}' with geometry", id))?;
    let result = stroke_outline::offset_d(&d, delta).map_err(|e| anyhow::anyhow!("{}", e))?;
    let out_name = out
        .map(|s| s.to_string())
        .unwrap_or_else(|| format!("{}-offset", id));
    let shape = stroke_outline::shapes_to_shape(&out_name, &result);
    append_generated_shape(path, &shape, &out_name)?;
    eprintln!("offset {} by {}: → shape/place '{}'", id, delta, out_name);
    Ok(())
}

/// Emit a generated `path` shape (document-coordinate) + a `place … at=0,0`
/// (identity) into the file, validating by re-parse. The shape DSL is produced
/// by the core emitter so it round-trips identically.
fn append_generated_shape(path: &Path, shape: &strok_core::shape::Shape, name: &str) -> Result<()> {
    if shape.operations.is_empty() {
        anyhow::bail!(
            "result geometry is empty (e.g. fully subtracted / disjoint intersect) — nothing written"
        );
    }
    let block = dsl_emit::emit_shape_block(shape);
    let place = format!("place {} shape={} at=0,0", name, name);
    let mut chunk = String::new();
    chunk.push_str(&block);
    if !chunk.ends_with('\n') {
        chunk.push('\n');
    }
    chunk.push_str(&place);
    append_line(path, &chunk)?;
    Ok(())
}

/// Append a DSL line to a .strok file, then validate by re-parsing.
fn append_line(path: &Path, line: &str) -> Result<()> {
    let existing = if path.exists() {
        std::fs::read_to_string(path)?
    } else {
        String::new()
    };

    let mut new_content = existing;
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }
    // Don't add extra newline if the line already ends with one
    if line.ends_with('\n') {
        new_content.push_str(line);
    } else {
        new_content.push_str(line);
        new_content.push('\n');
    }

    // Validate
    dsl_parse::parse_file_with_path(&new_content, path).with_context(|| {
        format!(
            "invalid DSL — the appended line was rejected:\n\n  {}\n",
            line.trim()
        )
    })?;

    std::fs::write(path, &new_content)?;
    Ok(())
}

/// Reorder a place block within a .strok file.
/// Pure text manipulation — moves the block identified by `name` to
/// front, back, before=<target>, or after=<target>.
fn reorder_block(path: &Path, name: &str, position: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let raw_lines: Vec<&str> = content.lines().collect();

    // Identify "blocks": a top-level `place <name>` line + all following indented lines
    struct Block {
        start: usize,
        end: usize, // exclusive
    }

    let mut blocks: Vec<(String, Block)> = Vec::new();
    let mut i = 0;
    while i < raw_lines.len() {
        let line = raw_lines[i];
        let trimmed = line.trim_start();
        // A top-level place line starts with "place " at indent 0
        if trimmed.starts_with("place ") && line.len() == trimmed.len() {
            // Extract the place name (second token)
            let block_name = trimmed.split_whitespace().nth(1).unwrap_or("").to_string();
            let start = i;
            i += 1;
            // Collect indented continuation lines
            while i < raw_lines.len() {
                let next = raw_lines[i];
                if next.is_empty() || next.starts_with("  ") || next.starts_with('\t') {
                    // Could be part of the block body or a blank separator
                    // Only include if it starts with whitespace (body line)
                    if next.starts_with("  ") || next.starts_with('\t') {
                        i += 1;
                    } else {
                        break;
                    }
                } else {
                    break;
                }
            }
            blocks.push((block_name, Block { start, end: i }));
        } else {
            i += 1;
        }
    }

    // Find the source block
    let place_names: Vec<&str> = blocks.iter().map(|(n, _)| n.as_str()).collect();
    let src_idx = blocks.iter().position(|(n, _)| n == name).ok_or_else(|| {
        anyhow::anyhow!(
            "no place named '{}' in file\n\nPlaces in this file: {}",
            name,
            if place_names.is_empty() {
                "(none)".to_string()
            } else {
                place_names.join(", ")
            }
        )
    })?;

    // Extract source lines
    let src_block = &blocks[src_idx].1;
    let src_lines: Vec<&str> = raw_lines[src_block.start..src_block.end].to_vec();

    // Build new file without the source block
    let mut remaining: Vec<&str> = Vec::new();
    remaining.extend_from_slice(&raw_lines[..src_block.start]);
    remaining.extend_from_slice(&raw_lines[src_block.end..]);

    // Re-scan remaining for place blocks to find insertion point
    let mut place_ranges: Vec<(String, usize, usize)> = Vec::new();
    let mut j = 0;
    while j < remaining.len() {
        let line = remaining[j];
        let trimmed = line.trim_start();
        if trimmed.starts_with("place ") && line.len() == trimmed.len() {
            let block_name = trimmed.split_whitespace().nth(1).unwrap_or("").to_string();
            let start = j;
            j += 1;
            while j < remaining.len() {
                let next = remaining[j];
                if next.starts_with("  ") || next.starts_with('\t') {
                    j += 1;
                } else {
                    break;
                }
            }
            place_ranges.push((block_name, start, j));
        } else {
            j += 1;
        }
    }

    let insert_at = if position == "front" {
        // After all place blocks (last in file = highest z)
        remaining.len()
    } else if position == "back" {
        // Before the first place block
        place_ranges
            .first()
            .map(|(_, start, _)| *start)
            .unwrap_or(remaining.len())
    } else if let Some(target) = position.strip_prefix("before=") {
        let remaining_names: Vec<&str> = place_ranges.iter().map(|(n, _, _)| n.as_str()).collect();
        place_ranges
            .iter()
            .find(|(n, _, _)| n == target)
            .map(|(_, start, _)| *start)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no place named '{}' to position relative to\n\nPlaces in this file: {}",
                    target,
                    if remaining_names.is_empty() {
                        "(none)".to_string()
                    } else {
                        remaining_names.join(", ")
                    }
                )
            })?
    } else if let Some(target) = position.strip_prefix("after=") {
        let remaining_names: Vec<&str> = place_ranges.iter().map(|(n, _, _)| n.as_str()).collect();
        place_ranges
            .iter()
            .find(|(n, _, _)| n == target)
            .map(|(_, _, end)| *end)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "no place named '{}' to position relative to\n\nPlaces in this file: {}",
                    target,
                    if remaining_names.is_empty() {
                        "(none)".to_string()
                    } else {
                        remaining_names.join(", ")
                    }
                )
            })?
    } else {
        anyhow::bail!(
            "invalid position '{}' (expected front, back, before=<name>, after=<name>)\n\nExamples:\n  strok reorder badge front\n  strok reorder glow before=badge",
            position
        );
    };

    // Insert
    let mut result: Vec<&str> = Vec::new();
    result.extend_from_slice(&remaining[..insert_at]);
    result.extend_from_slice(&src_lines);
    result.extend_from_slice(&remaining[insert_at..]);

    let mut new_content = result.join("\n");
    if !new_content.ends_with('\n') {
        new_content.push('\n');
    }

    // Validate
    dsl_parse::parse_file_with_path(&new_content, path)
        .with_context(|| format!("invalid DSL after reordering '{}'", name))?;

    std::fs::write(path, &new_content)?;
    eprintln!("reordered {} to {}", name, position);
    Ok(())
}

/// Insert an operation line into an existing shape block.
/// Finds the shape block by name and appends the line before the next
/// top-level statement.
fn insert_into_shape_block(path: &Path, shape_name: &str, op_line: &str) -> Result<()> {
    let content = std::fs::read_to_string(path)?;
    let mut lines: Vec<&str> = content.lines().collect();

    // Find the shape block header
    let shape_marker = format!("shape {} ", shape_name);
    let mut insert_at = None;

    for (i, line) in lines.iter().enumerate() {
        if line.starts_with(&shape_marker) || line == &format!("shape {}", shape_name) {
            // Found the shape block. Now find where it ends (next non-indented line).
            for (j, l) in lines.iter().enumerate().skip(i + 1) {
                if !l.starts_with("  ") && !l.is_empty() {
                    insert_at = Some(j);
                    break;
                }
            }
            if insert_at.is_none() {
                insert_at = Some(lines.len());
            }
            break;
        }
    }

    let insert_at =
        insert_at.ok_or_else(|| anyhow::anyhow!("shape '{}' not found in file", shape_name))?;

    lines.insert(insert_at, op_line);

    let mut new_content = lines.join("\n");
    if !new_content.ends_with('\n') {
        new_content.push('\n');
    }

    // Validate
    dsl_parse::parse_file_with_path(&new_content, path)
        .with_context(|| format!("invalid DSL after inserting into shape '{}'", shape_name))?;

    std::fs::write(path, &new_content)?;
    Ok(())
}

#[cfg(test)]
mod visual_grammar_tests {
    use super::*;

    #[test]
    fn explicit_icon_profiles_classify_as_authored() {
        for (profile, expected) in [
            ("icon-outline-round", "outline-round"),
            ("icon-outline-angular", "outline-angular"),
            ("icon-solid", "solid"),
        ] {
            let source = build_document_source("24x24", Some(profile)).unwrap();
            let scene = dsl_parse::parse_file(&source).unwrap();
            assert_eq!(classify_visual_grammar(&scene), expected, "{profile}");
        }
    }

    #[test]
    fn solid_mass_with_stroked_detail_classifies_as_mixed() {
        let source = "\
documentsize 24x24
defaults
  fill currentColor
  stroke none
shape mass template=rectangle
shape detail template=line
  fill none
  stroke currentColor
  stroke-width 2
place mass shape=mass at=3,3 size=18x18
place detail shape=detail at=7,12 size=10x0
";
        let scene = dsl_parse::parse_file(source).unwrap();
        assert_eq!(classify_visual_grammar(&scene), "mixed");
    }

    #[test]
    fn per_shape_round_outline_classifies_without_defaults_block() {
        let source = "\
documentsize 24x24
shape mark template=path
  fill none
  stroke #17150f
  stroke-width 2
  stroke-linecap round
  stroke-linejoin round
  addpoint a at=4,12
  addpoint b at=20,12
place mark shape=mark at=0,0
";
        let scene = dsl_parse::parse_file(source).unwrap();
        assert_eq!(classify_visual_grammar(&scene), "outline-round");
    }
}
