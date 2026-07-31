/// v3 emitter — Scene → .strok DSL text.
///
/// Round-trip: parse(emit(scene)) == scene.
use crate::scene::*;
use crate::shape::*;
use crate::types::*;

/// Emit a single `shape` block as .strok DSL text (header + operations +
/// effects). Used by the C3 boolean / outline / offset commands to write the
/// generated path shape; the output parses back to an identical `Shape`.
pub fn emit_shape_block(shape: &Shape) -> String {
    let mut out = String::new();
    emit_shape(shape, &mut out);
    out
}

/// Emit a Scene as .strok DSL text.
pub fn emit_scene(scene: &Scene) -> String {
    let mut out = String::new();

    out.push_str(&format!("documentsize {}\n", scene.document_size));

    // Emit imports
    for import in &scene.imports {
        if let Some(ns) = &import.namespace {
            out.push_str(&format!("\nuse \"{}\" as {}", import.path, ns));
        } else {
            out.push_str(&format!("\nuse \"{}\"", import.path));
        }
        out.push('\n');
    }

    // Emit palette and scheme blocks
    if !scene.palette.tokens.is_empty() {
        out.push_str("\npalette\n");
        for (name, color) in &scene.palette.tokens {
            out.push_str(&format!("  {} {}\n", name, color));
        }
    }
    for sc in &scene.palette.schemes {
        out.push_str(&format!("\nscheme {}\n", sc.name));
        for (name, color) in &sc.tokens {
            out.push_str(&format!("  {} {}\n", name, color));
        }
    }

    // Generalized design tokens (C8 / E4.1): emitted as a `tokens` block with
    // dotted `category.name value` entries, in declaration order.
    if !scene.design_tokens.is_empty() {
        out.push_str("\ntokens\n");
        for t in &scene.design_tokens {
            out.push_str(&format!("  {} {}\n", t.dotted(), t.value));
        }
    }

    // `let` bindings (C13): re-emit `let <name> <expr-source>` in declaration
    // order, before shapes. The source string is kept verbatim so the round-trip
    // invariant holds even though the scene otherwise stores only evaluated
    // numbers. NOTE: a `repeat` block does *not* survive — its expanded nodes are
    // what the scene holds — so any CLI command that re-emits the file flattens
    // repeats to their concrete nodes (documented in DSL_SPEC § repeat).
    if !scene.lets.is_empty() {
        out.push('\n');
        for (name, source, _value) in &scene.lets {
            out.push_str(&format!("let {} {}\n", name, source));
        }
    }

    // Emit defaults block
    if !scene.defaults.is_empty() {
        out.push_str("\ndefaults\n");
        for op in &scene.defaults {
            out.push_str("  ");
            emit_operation(op, &mut out);
            out.push('\n');
        }
    }

    // A `createlink` is parsed into BOTH a `Link` node (the authoritative
    // representation, carrying source + overrides + effects) AND a placeable
    // `Shape` alias. We must emit each createlink exactly ONCE: the `Link` node
    // emission (in `emit_node`) is the single source of truth. Emitting the
    // alias shape *as well* duplicated every top-level createlink, and the
    // duplication compounded on each re-parse — breaking the round-trip
    // invariant (caught by `fuzz_roundtrip` on examples/shape-language.strok).
    //
    // So: emit only the *base* shapes here (those NOT referenced by any Link
    // node) and let the node loop emit the links.
    let link_names: Vec<String> = scene
        .nodes
        .iter()
        .filter_map(find_link_sources)
        .flatten()
        .collect();

    for shape in &scene.shapes {
        if link_names.contains(&shape.name) {
            continue; // emitted via its Link node below
        }
        // A shape merged in from a `use` import (filesystem or embedded
        // `std/` module, EXP-1) is re-emitted via its `use` line above, not
        // inlined here — otherwise every re-save of an importing document
        // would duplicate the whole imported module into the file.
        if scene.imported_shape_names.contains(&shape.name) {
            continue;
        }
        out.push('\n');
        emit_shape(shape, &mut out);
    }

    // Emit component definitions (C8 / E4.2) before the nodes that instance them.
    for component in &scene.components {
        out.push('\n');
        emit_component(component, &mut out);
    }

    // Emit scene nodes (Place / Group / Link / Frame / Instance). Link nodes
    // emit their createlink.
    for node in &scene.nodes {
        out.push('\n');
        emit_node(node, 0, &mut out);
    }

    out
}

/// Emit a `component` block (C8 / E4.2).
fn emit_component(c: &Component, out: &mut String) {
    let mut line = format!("component {}", c.name);
    if !c.variants.is_empty() {
        line.push_str(&format!(" variants=[{}]", c.variants.join(", ")));
    }
    if !c.props.is_empty() {
        let props: Vec<String> = c
            .props
            .iter()
            .map(|(n, ty)| format!("{}:{}", n, ty))
            .collect();
        line.push_str(&format!(" props=[{}]", props.join(", ")));
    }
    line.push('\n');
    out.push_str(&line);
    for child in &c.children {
        emit_node(child, 1, out);
    }
}

/// Find Link sources from scene nodes.
fn find_link_sources(node: &SceneNode) -> Option<Vec<String>> {
    match node {
        SceneNode::Link(l) => Some(vec![l.name.clone()]),
        SceneNode::Group(g) => {
            let mut names = Vec::new();
            for child in &g.children {
                if let Some(child_names) = find_link_sources(child) {
                    names.extend(child_names);
                }
            }
            if names.is_empty() {
                None
            } else {
                Some(names)
            }
        }
        SceneNode::Boolean(b) => {
            let mut names = Vec::new();
            for child in &b.children {
                if let Some(child_names) = find_link_sources(child) {
                    names.extend(child_names);
                }
            }
            if names.is_empty() {
                None
            } else {
                Some(names)
            }
        }
        _ => None,
    }
}

fn emit_shape(shape: &Shape, out: &mut String) {
    out.push_str(&format!(
        "shape {} template={}\n",
        shape.name, shape.template
    ));

    for op in &shape.operations {
        out.push_str("  ");
        emit_operation(op, out);
        out.push('\n');
    }

    for effect in &shape.effects {
        out.push_str("  ");
        emit_effect(effect, out);
        out.push('\n');
    }
}

fn emit_operation(op: &Operation, out: &mut String) {
    match op {
        Operation::MovePointDelta { point, dx, dy } => {
            out.push_str(&format!(
                "movepoint {} dx={} dy={}",
                point,
                fmt(*dx),
                fmt(*dy)
            ));
        }
        Operation::MovePointTo { point, to } => {
            out.push_str(&format!(
                "movepoint {} to={},{}",
                point,
                fmt(to.0),
                fmt(to.1)
            ));
        }
        Operation::PullPointDir { point, dir, amount } => {
            out.push_str(&format!("pullpoint {} dir={} {}", point, dir, amount));
        }
        Operation::PullPointDelta {
            point,
            dx,
            dy,
            radius,
            falloff,
        } => {
            out.push_str(&format!(
                "pullpoint {} dx={} dy={} radius={} falloff={}",
                point,
                fmt(*dx),
                fmt(*dy),
                radius,
                falloff
            ));
        }
        Operation::Sculpt {
            at,
            dx,
            dy,
            radius,
            falloff,
            axis,
            lock_endpoints,
        } => {
            out.push_str("sculpt");
            match at {
                SculptTarget::Point(pr) => out.push_str(&format!(" at={}", pr)),
                SculptTarget::Coord(x, y) => out.push_str(&format!(" at={},{}", fmt(*x), fmt(*y))),
                SculptTarget::Segment(sr) => out.push_str(&format!(" at={}", sr)),
            }
            out.push_str(&format!(" dx={} dy={}", fmt(*dx), fmt(*dy)));
            if let Some(r) = radius {
                out.push_str(&format!(" radius={}", r));
            }
            if let Some(f) = falloff {
                out.push_str(&format!(" falloff={}", f));
            }
            if let Some(a) = axis {
                out.push_str(&format!(" axis={}", a));
            }
            if *lock_endpoints {
                out.push_str(" lock-endpoints");
            }
        }
        Operation::AddPoint {
            name,
            at,
            after,
            mode,
            tension,
            arc_rx,
            arc_ry,
            arc_sweep,
            arc_large,
            arc_bulge,
            control_c1,
            control_c2,
        } => {
            out.push_str(&format!("addpoint {} at={},{}", name, fmt(at.0), fmt(at.1)));
            if let Some(after_name) = after {
                out.push_str(&format!(" after={}", after_name));
            }
            if let Some(m) = mode {
                out.push_str(&format!(" mode={}", m));
            }
            if let Some(t) = tension {
                out.push_str(&format!(" tension={}", fmt(*t)));
            }
            if let Some(rx) = arc_rx {
                out.push_str(&format!(" rx={}", fmt(*rx)));
            }
            if let Some(ry) = arc_ry {
                out.push_str(&format!(" ry={}", fmt(*ry)));
            }
            if let Some(sweep) = arc_sweep {
                out.push_str(&format!(" sweep={}", if *sweep { "1" } else { "0" }));
            }
            if let Some(large) = arc_large {
                out.push_str(&format!(" large={}", if *large { "1" } else { "0" }));
            }
            if let Some(bulge) = arc_bulge {
                out.push_str(&format!(" bulge={}", bulge));
            }
            if let Some(c1) = control_c1 {
                out.push_str(&format!(" c1={},{}", fmt(c1.0), fmt(c1.1)));
            }
            if let Some(c2) = control_c2 {
                out.push_str(&format!(" c2={},{}", fmt(c2.0), fmt(c2.1)));
            }
        }
        Operation::SplitLine { segment, name, t } => {
            out.push_str(&format!("splitline {} name={}", segment, name));
            if let Some(t_val) = t {
                out.push_str(&format!(" t={}", t_val));
            }
        }
        Operation::DeletePoint { point, reconnect } => {
            out.push_str(&format!("deletepoint {}", point));
            if let Some(r) = reconnect {
                out.push_str(&format!(" reconnect={}", r));
            }
        }
        Operation::Close => out.push_str("close"),
        Operation::Open => out.push_str("open"),
        Operation::Subpath => out.push_str("subpath"),
        Operation::Smooth { point, tension } => {
            out.push_str(&format!("smooth {}", point));
            if let Some(t) = tension {
                out.push_str(&format!(" tension={}", fmt(*t)));
            }
        }
        Operation::SmoothCorner { point, tension } => {
            out.push_str(&format!("smooth-corner {}", point));
            if let Some(t) = tension {
                out.push_str(&format!(" tension={}", fmt(*t)));
            }
        }
        Operation::Sharpen { point } => {
            out.push_str(&format!("sharpen {}", point));
        }
        Operation::SmoothAll { tension } => {
            out.push_str("smooth all");
            if let Some(t) = tension {
                out.push_str(&format!(" tension={}", fmt(*t)));
            }
        }
        Operation::SharpenAll => {
            out.push_str("sharpen all");
        }
        Operation::ConvertPoint { point, to } => {
            out.push_str(&format!("convert-point {} to={}", point, to.as_str()));
        }
        Operation::RoundCorners { radii } => match radii {
            CornerRadii::Uniform(r) => {
                out.push_str(&format!("round-corners {}", fmt(*r)));
            }
            CornerRadii::PerCorner(list) => {
                out.push_str("round-corners");
                for (name, r) in list {
                    out.push_str(&format!(" {}={}", name, fmt(*r)));
                }
            }
        },
        Operation::Notch(spec) => {
            let edge = match &spec.edge {
                NotchEdge::Named(e) => e.as_str().to_string(),
                NotchEdge::Segment(a, b) => format!("{},{}", a, b),
            };
            out.push_str(&format!(
                "notch edge={} dir={} shape={} pos={} width={} depth={}",
                edge,
                spec.dir.as_str(),
                spec.shape.as_str(),
                fmt(spec.pos),
                fmt(spec.width),
                fmt(spec.depth),
            ));
        }
        Operation::Fill(color) => {
            out.push_str(&format!("fill {}", color));
        }
        Operation::FillRule(rule) => {
            out.push_str(&format!("fill-rule {}", rule));
        }
        Operation::Stroke(color) => {
            out.push_str(&format!("stroke {}", color));
        }
        Operation::StrokeWidth(w) => {
            out.push_str(&format!("stroke-width {}", w));
        }
        Operation::StrokeLinecap(c) => {
            out.push_str(&format!("stroke-linecap {}", c));
        }
        Operation::StrokeLinejoin(j) => {
            out.push_str(&format!("stroke-linejoin {}", j));
        }
        Operation::StrokeMiterlimit(m) => {
            out.push_str(&format!("stroke-miterlimit {}", fmt(*m)));
        }
        Operation::StrokeDasharray(values) => {
            out.push_str("stroke-dasharray");
            for v in values {
                out.push_str(&format!(" {}", fmt(*v)));
            }
        }
        Operation::Opacity(a) => {
            out.push_str(&format!("opacity {}", a));
        }
        Operation::Blur(r) => {
            out.push_str(&format!("blur {}", fmt(*r)));
        }
        Operation::Content(text) => {
            out.push_str(&format!("content \"{}\"", text));
        }
        Operation::FontSize(v) => {
            out.push_str(&format!("font-size {}", fmt(*v)));
        }
        Operation::FontFamily(s) => {
            out.push_str(&format!("font-family \"{}\"", s));
        }
        Operation::FontWeight(s) => {
            out.push_str(&format!("font-weight {}", s));
        }
        Operation::FontStyle(s) => {
            out.push_str(&format!("font-style {}", s));
        }
        Operation::TextAnchor(a) => {
            out.push_str(&format!("text-anchor {}", a));
        }
    }
}

fn emit_effect(effect: &Effect, out: &mut String) {
    match effect {
        Effect::Droop { amount, direction } => {
            out.push_str(&format!("applyeffect droop {}", amount));
            if let Some(dir) = direction {
                out.push_str(&format!(" direction={}", dir));
            }
        }
        Effect::Curl { amount, from } => {
            out.push_str(&format!("applyeffect curl {}", amount));
            if let Some(pr) = from {
                out.push_str(&format!(" from={}", pr));
            }
        }
        Effect::Taper { start, end } => {
            out.push_str(&format!("applyeffect taper start={} end={}", start, end));
        }
        Effect::Jitter { amount, seed } => {
            out.push_str(&format!("applyeffect jitter {}", amount));
            if let Some(s) = seed {
                out.push_str(&format!(" seed={}", s));
            }
        }
    }
}

fn emit_node(node: &SceneNode, indent: usize, out: &mut String) {
    let prefix = "  ".repeat(indent);
    match node {
        SceneNode::Place(p) => {
            out.push_str(&format!("{}place {} shape={}", prefix, p.name, p.shape_ref));
            emit_place_position(&p.position, out);
            if let Some(size) = &p.size {
                out.push_str(&format!(" size={}", size));
            }
            if let Some(rot) = &p.rotation {
                out.push_str(&format!(" rotation={}", rot));
            }
            if let Some(flip) = &p.flip {
                out.push_str(&format!(" flip={}", flip));
            }
            if let Some((sx, sy)) = &p.skew {
                out.push_str(&format!(" skew={}", emit_skew(*sx, *sy)));
            }
            if let Some(clip) = &p.clip {
                out.push_str(&format!(" clip={}", clip.join(",")));
            }
            if let Some(mask) = &p.mask {
                out.push_str(&format!(" mask={}", mask));
            }
            if let Some(align) = &p.align {
                out.push_str(&format!(" align={}", align));
            }
            if let Some((dx, dy)) = &p.offset {
                out.push_str(&format!(" offset={},{}", fmt(*dx), fmt(*dy)));
            }
            if let Some(tp) = &p.text_path {
                out.push_str(&format!(" textpath={}", tp));
            }
            if let Some(anchor) = &p.anchor {
                match anchor {
                    PlaceAnchor::Below { target, gap } => {
                        out.push_str(&format!(" below={}", target));
                        if *gap != 0.0 {
                            out.push_str(&format!(" gap={}", fmt(*gap)));
                        }
                    }
                    PlaceAnchor::Above { target, gap } => {
                        out.push_str(&format!(" above={}", target));
                        if *gap != 0.0 {
                            out.push_str(&format!(" gap={}", fmt(*gap)));
                        }
                    }
                }
            }
            out.push('\n');

            // Inline overrides
            for op in &p.overrides {
                out.push_str(&format!("{}  ", prefix));
                emit_operation(op, out);
                out.push('\n');
            }
        }
        SceneNode::Group(g) => {
            let mut group_line = format!("{}group {}", prefix, g.name);
            if let Some((x, y)) = g.position {
                group_line.push_str(&format!(" at={},{}", fmt(x), fmt(y)));
            }
            if let Some(rot) = &g.rotation {
                group_line.push_str(&format!(" rotation={}", rot));
            }
            if let Some(flip) = &g.flip {
                group_line.push_str(&format!(" flip={}", flip));
            }
            if let Some((sx, sy)) = &g.skew {
                group_line.push_str(&format!(" skew={}", emit_skew(*sx, *sy)));
            }
            if let Some(clip) = &g.clip {
                group_line.push_str(&format!(" clip={}", clip.join(",")));
            }
            if let Some(mask) = &g.mask {
                group_line.push_str(&format!(" mask={}", mask));
            }
            if let Some(opacity) = g.opacity {
                group_line.push_str(&format!(" opacity={}", fmt(opacity)));
            }
            group_line.push('\n');
            out.push_str(&group_line);
            for child in &g.children {
                emit_node(child, indent + 1, out);
            }
        }
        SceneNode::Boolean(b) => {
            out.push_str(&format!(
                "{}boolean {} op={}\n",
                prefix,
                b.name,
                b.op.name()
            ));
            for child in &b.children {
                emit_node(child, indent + 1, out);
            }
            for op in &b.operations {
                out.push_str(&format!("{}  ", prefix));
                emit_operation(op, out);
                out.push('\n');
            }
        }
        SceneNode::Link(l) => {
            out.push_str(&format!(
                "{}createlink {} from={}\n",
                prefix, l.name, l.source
            ));
            for op in &l.overrides {
                out.push_str(&format!("{}  ", prefix));
                emit_operation(op, out);
                out.push('\n');
            }
            for effect in &l.effects {
                out.push_str(&format!("{}  ", prefix));
                emit_effect(effect, out);
                out.push('\n');
            }
        }
        SceneNode::Frame(fr) => emit_frame(fr, indent, out),
        SceneNode::Instance(i) => emit_instance(i, indent, out),
    }
}

/// Emit a `frame` block (C8 / E4.1).
fn emit_frame(fr: &Frame, indent: usize, out: &mut String) {
    let prefix = "  ".repeat(indent);
    let mut line = format!("{}frame {}", prefix, fr.name);
    let layout = emit_layout(&fr.layout);
    if !layout.is_empty() {
        line.push_str(&format!(" layout={}", layout));
    }
    if let Some(size) = &fr.size {
        line.push_str(&format!(" size={}", size));
    }
    if let Some((x, y)) = fr.position {
        line.push_str(&format!(" at={},{}", fmt(x), fmt(y)));
    }
    line.push('\n');
    out.push_str(&line);

    if let Some(fill) = &fr.fill {
        out.push_str(&format!("{}  fill {}\n", prefix, fill));
    }
    if let Some(radius) = &fr.radius {
        match radius {
            RadiusValue::Literal(r) => {
                out.push_str(&format!("{}  round-corners {}\n", prefix, fmt(*r)))
            }
            RadiusValue::Token(t) => out.push_str(&format!("{}  round-corners ${}\n", prefix, t)),
        }
    }
    if let Some(o) = fr.opacity {
        out.push_str(&format!("{}  opacity {}\n", prefix, fmt(o)));
    }
    for child in &fr.children {
        emit_node(child, indent + 1, out);
    }
}

/// Emit an `instance` line (C8 / E4.2).
fn emit_instance(i: &Instance, indent: usize, out: &mut String) {
    let prefix = "  ".repeat(indent);
    let mut line = format!("{}instance {} from={}", prefix, i.name, i.component);
    if let Some(v) = &i.variant {
        line.push_str(&format!(" variant={}", v));
    }
    for (k, val) in &i.props {
        line.push_str(&format!(" {}={}", k, emit_prop_value(val)));
    }
    if let Some((x, y)) = i.position {
        line.push_str(&format!(" at={},{}", fmt(x), fmt(y)));
    }
    if let Some(size) = &i.size {
        line.push_str(&format!(" size={}", size));
    }
    line.push('\n');
    out.push_str(&line);
}

/// Serialize a [`Layout`] back to its DSL `layout=` value.
fn emit_layout(layout: &Layout) -> String {
    match layout {
        Layout::None => String::new(),
        Layout::Flow => "flow".to_string(),
        Layout::Flex {
            direction,
            gap,
            padding,
            align,
            justify,
        } => {
            let mut parts = Vec::new();
            parts.push(match direction {
                FlexDirection::Row => "row".to_string(),
                FlexDirection::Col => "col".to_string(),
            });
            if *gap != 0.0 {
                parts.push(format!("gap={}", fmt(*gap)));
            }
            let (t, r, b, l) = *padding;
            if !(t == 0.0 && r == 0.0 && b == 0.0 && l == 0.0) {
                parts.push(format!("padding={}", emit_padding(t, r, b, l)));
            }
            if !matches!(align, FlexAlign::Start) {
                parts.push(format!(
                    "align={}",
                    match align {
                        FlexAlign::Start => "start",
                        FlexAlign::Center => "center",
                        FlexAlign::End => "end",
                        FlexAlign::Stretch => "stretch",
                    }
                ));
            }
            if !matches!(justify, FlexJustify::Start) {
                parts.push(format!(
                    "justify={}",
                    match justify {
                        FlexJustify::Start => "start",
                        FlexJustify::Center => "center",
                        FlexJustify::End => "end",
                        FlexJustify::Between => "between",
                    }
                ));
            }
            format!("flex({})", parts.join(", "))
        }
        Layout::Grid { columns, gap } => {
            if *gap != 0.0 {
                format!("grid(columns={}, gap={})", columns, fmt(*gap))
            } else {
                format!("grid(columns={})", columns)
            }
        }
    }
}

/// Emit padding in the most compact equivalent form (`N` / `x,y` / `t,r,b,l`).
fn emit_padding(t: f64, r: f64, b: f64, l: f64) -> String {
    if t == r && r == b && b == l {
        fmt(t)
    } else if t == b && l == r {
        format!("{},{}", fmt(r), fmt(t)) // x,y
    } else {
        format!("{},{},{},{}", fmt(t), fmt(r), fmt(b), fmt(l))
    }
}

/// Quote a prop value if it contains whitespace (so it round-trips as one token).
fn emit_prop_value(v: &str) -> String {
    if v.chars().any(|c| c.is_whitespace()) {
        format!("\"{}\"", v)
    } else {
        v.to_string()
    }
}

fn emit_place_position(pos: &PlacePosition, out: &mut String) {
    match pos {
        PlacePosition::At(x, y) => {
            out.push_str(&format!(" at={},{}", fmt(*x), fmt(*y)));
        }
        PlacePosition::On {
            path,
            t,
            side,
            offset,
        } => {
            out.push_str(&format!(" on={} at={}", path, t));
            if let Some(s) = side {
                out.push_str(&format!(" side={}", s));
            }
            if let Some(o) = offset {
                out.push_str(&format!(" offset={}", o));
            }
        }
        PlacePosition::RelativeTo { target, anchor } => {
            out.push_str(&format!(" at={}.{}", target, anchor));
        }
    }
}

/// Format a number: trim trailing zeros, avoid "-0".
fn fmt(n: f64) -> String {
    fmt_num(n)
}

/// Emit a `skew=` value: a single number when the y-skew is zero, else `x,y`.
fn emit_skew(sx: f64, sy: f64) -> String {
    if sy == 0.0 {
        fmt(sx)
    } else {
        format!("{},{}", fmt(sx), fmt(sy))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::dsl_parse;

    #[test]
    fn round_trip_minimal() {
        let input = "\
documentsize 400x400

shape bg template=rectangle
  fill #faf6f0

place bg shape=bg at=0,0 size=400x400
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(scene.document_size, scene2.document_size);
        assert_eq!(scene.shapes.len(), scene2.shapes.len());
        assert_eq!(scene.nodes.len(), scene2.nodes.len());
    }

    #[test]
    fn round_trip_with_operations() {
        let input = "\
documentsize 400x400

shape stem template=path
  addpoint base at=200,385
  addpoint mid at=192,300 after=base mode=catmull-rom tension=0.3
  smooth-corner mid tension=0.25
  stroke #3a7d44
  stroke-width 5
  stroke-linecap round

place stem shape=stem at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(
            scene.shapes[0].operations.len(),
            scene2.shapes[0].operations.len()
        );
        assert!(output.contains("smooth-corner mid tension=0.25"));
    }

    #[test]
    fn round_trip_with_effects() {
        let input = "\
documentsize 400x400

shape petal template=ellipse
  pullpoint top dir=up 15%
  pullpoint bottom dir=down 5%
  applyeffect droop 0.15

place petal shape=petal at=200,150 size=60x100
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(
            scene.shapes[0].effects.len(),
            scene2.shapes[0].effects.len()
        );
    }

    #[test]
    fn round_trip_text_shape() {
        let input = "\
documentsize 400x200

shape title template=text
  content \"Sample\"
  font-size 52
  font-weight bold
  font-family \"Georgia\"
  fill #2c1810

place title shape=title at=100,130
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(scene.shapes[0].template, scene2.shapes[0].template);
        assert_eq!(
            scene.shapes[0].operations.len(),
            scene2.shapes[0].operations.len()
        );
        // Verify content survived
        assert_eq!(scene.shapes[0].content(), scene2.shapes[0].content());
    }

    #[test]
    fn round_trip_anchor() {
        let input = "\
documentsize 200x400

shape box template=rectangle
  fill #ff0000

place top shape=box at=10,10 size=100x50
place bottom shape=box at=10,0 below=top gap=5 size=100x50
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        if let SceneNode::Place(p) = &scene2.nodes[1] {
            assert!(p.anchor.is_some());
        } else {
            panic!("expected Place");
        }
    }

    #[test]
    fn round_trip_place_with_flip_and_rotation() {
        let input = "\
documentsize 400x400

shape leaf template=path
  addpoint tip at=0,0
  close

place leaf-r shape=leaf at=100,200 rotation=20deg flip=x
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        if let SceneNode::Place(p) = &scene2.nodes[0] {
            assert_eq!(p.rotation, Some(Rotation(20.0)));
            assert_eq!(p.flip, Some(Flip::X));
        } else {
            panic!("expected Place");
        }
    }

    #[test]
    fn round_trip_radial_gradient() {
        let input = "\
documentsize 400x400

shape glow template=ellipse
  fill radial(center, 80%, #ff6b6b, transparent)

place glow shape=glow at=100,100 size=200x200
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        let fill1 = scene.shapes[0].fill().unwrap();
        let fill2 = scene2.shapes[0].fill().unwrap();
        assert_eq!(fill1, fill2);
    }

    #[test]
    fn round_trip_linear_gradient() {
        let input = "\
documentsize 400x400

shape sky template=rectangle
  fill linear(top, bottom, #ff0000, #0000ff)

place sky shape=sky at=0,0 size=400x400
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        let fill1 = scene.shapes[0].fill().unwrap();
        let fill2 = scene2.shapes[0].fill().unwrap();
        assert_eq!(fill1, fill2);
    }

    #[test]
    fn round_trip_multistop_gradient() {
        let input = "\
documentsize 400x400

shape bar template=rectangle
  fill radial(center, 50%, #d8b480 0%, #c4a070 60%, transparent 100%)

place bar shape=bar at=0,0 size=400x400
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        let fill1 = scene.shapes[0].fill().unwrap();
        let fill2 = scene2.shapes[0].fill().unwrap();
        assert_eq!(fill1, fill2);
    }

    #[test]
    fn round_trip_relative_placement() {
        let input = "\
documentsize 800x600

shape box template=rectangle
  fill #ff0000

shape dot template=ellipse
  fill #0000ff

place a shape=box at=100,100 size=200x100
place b shape=dot at=a.right align=left offset=5,-3 size=50x50
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        if let SceneNode::Place(p) = &scene2.nodes[1] {
            assert_eq!(p.name, "b");
            match &p.position {
                PlacePosition::RelativeTo { target, anchor } => {
                    assert_eq!(target, "a");
                    assert_eq!(*anchor, crate::scene::BboxAnchor::Right);
                }
                _ => panic!("expected RelativeTo"),
            }
            assert_eq!(p.align, Some(crate::scene::BboxAnchor::Left));
            assert_eq!(p.offset, Some((5.0, -3.0)));
        } else {
            panic!("expected Place");
        }
    }

    #[test]
    fn round_trip_blur() {
        let input = "\
documentsize 400x400

shape shadow template=ellipse
  fill #000000
  blur 5

place shadow shape=shadow at=100,100 size=200x200
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(scene2.shapes[0].blur(), Some(5.0));
    }

    #[test]
    fn round_trip_group_clip_and_opacity() {
        let input = "\
documentsize 400x400

shape mask template=ellipse
shape dot template=ellipse

group eye clip=mask opacity=0.7
  place dot shape=dot at=0,0 size=10x10
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        if let SceneNode::Group(g) = &scene2.nodes[0] {
            assert_eq!(g.clip, Some(vec!["mask".to_string()]));
            assert_eq!(g.opacity, Some(0.7));
        } else {
            panic!("expected Group");
        }
    }

    #[test]
    fn round_trip_group_with_transforms() {
        let input = "\
documentsize 800x600

shape ring template=ellipse

group compass at=790,130 rotation=15deg flip=x
  place ring shape=ring at=0,0 size=200x200
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        if let SceneNode::Group(g) = &scene2.nodes[0] {
            assert_eq!(g.position, Some((790.0, 130.0)));
            assert_eq!(g.rotation, Some(crate::types::Rotation(15.0)));
            assert_eq!(g.flip, Some(crate::types::Flip::X));
        } else {
            panic!("expected Group");
        }
    }

    #[test]
    fn round_trip_group_no_transforms() {
        let input = "\
documentsize 400x400

shape dot template=ellipse

group plain
  place dot shape=dot at=0,0 size=10x10
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        if let SceneNode::Group(g) = &scene2.nodes[0] {
            assert_eq!(g.position, None);
            assert_eq!(g.rotation, None);
            assert_eq!(g.flip, None);
            assert_eq!(g.children.len(), 1);
        } else {
            panic!("expected Group");
        }
    }

    #[test]
    fn round_trip_use() {
        let input = "\
documentsize 400x400

use \"./eye.strok\"
use \"./face.strok\" as face

shape bg template=rectangle

place bg shape=bg at=0,0 size=400x400
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(scene2.imports.len(), 2);
        assert_eq!(scene2.imports[0].path, "./eye.strok");
        assert_eq!(scene2.imports[0].namespace, None);
        assert_eq!(scene2.imports[1].path, "./face.strok");
        assert_eq!(scene2.imports[1].namespace, Some("face".to_string()));
    }

    #[test]
    fn round_trip_palette_and_scheme() {
        let input = "\
documentsize 64x64

palette
  hero #e8a840
  accent #c8863a

scheme dark
  hero #f4c266
  accent #daa05a

shape bg template=rectangle
  fill $accent

place bg shape=bg at=0,0 size=64x64
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(scene.palette, scene2.palette);
        // The $token fill survives the round-trip.
        assert!(matches!(scene2.shapes[0].fill(), Some(Color::Token(t)) if t == "accent"));
    }

    #[test]
    fn round_trip_defaults() {
        let input = "\
documentsize 400x400

defaults
  fill #2d5a1e
  stroke #1a3a12
  stroke-width 1.5
  opacity 0.9

shape leaf template=ellipse

place leaf shape=leaf at=50,50 size=40x60
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(scene2.defaults.len(), 4);
    }

    #[test]
    fn round_trip_stroke_dasharray() {
        let input = "\
documentsize 400x400

shape border template=rectangle
  stroke #333333
  stroke-dasharray 5 3

place border shape=border at=0,0 size=400x300
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(scene2.shapes[0].stroke_dasharray(), Some(&[5.0, 3.0][..]));
    }

    #[test]
    fn round_trip_arc_with_flags() {
        let input = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,100
  addpoint end at=100,0 mode=arc rx=50 ry=30 sweep=0 large=1

place arc shape=arc at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        if let crate::shape::Operation::AddPoint {
            mode,
            arc_rx,
            arc_ry,
            arc_sweep,
            arc_large,
            ..
        } = &scene2.shapes[0].operations[1]
        {
            assert_eq!(*mode, Some(crate::types::PointMode::Arc));
            assert_eq!(*arc_rx, Some(50.0));
            assert_eq!(*arc_ry, Some(30.0));
            assert_eq!(*arc_sweep, Some(false));
            assert_eq!(*arc_large, Some(true));
        } else {
            panic!("expected AddPoint");
        }
    }

    #[test]
    fn round_trip_arc_defaults_omitted() {
        let input = "\
documentsize 400x400

shape arc template=path
  addpoint start at=0,100
  addpoint end at=100,0 mode=arc rx=50

place arc shape=arc at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        if let crate::shape::Operation::AddPoint {
            arc_rx,
            arc_ry,
            arc_sweep,
            arc_large,
            ..
        } = &scene2.shapes[0].operations[1]
        {
            assert_eq!(*arc_rx, Some(50.0));
            assert_eq!(*arc_ry, None);
            assert_eq!(*arc_sweep, None);
            assert_eq!(*arc_large, None);
        } else {
            panic!("expected AddPoint");
        }
    }

    #[test]
    fn round_trip_controls_mode() {
        let input = "\
documentsize 400x400

shape curve template=path
  addpoint start at=0,0
  addpoint end at=100,0 mode=controls c1=30,0 c2=80,40

place curve shape=curve at=0,0
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        if let crate::shape::Operation::AddPoint {
            mode,
            control_c1,
            control_c2,
            ..
        } = &scene2.shapes[0].operations[1]
        {
            assert_eq!(*mode, Some(crate::types::PointMode::Controls));
            assert_eq!(*control_c1, Some((30.0, 0.0)));
            assert_eq!(*control_c2, Some((80.0, 40.0)));
        } else {
            panic!("expected AddPoint");
        }
    }

    // ── C8: frame / layout / tokens / component / instance round-trip ──────

    /// Every C8 construct must survive `parse(emit(scene)) == scene` (the doc-02
    /// non-negotiable invariant).
    #[test]
    fn round_trip_c8_full() {
        let clean = "\
documentsize 320x200

palette
  surface #faf6f0
  accent #c8863a

tokens
  space.md 16
  radius.md 12
  radius.sm 6
  font.body \"IBM Plex Sans\"

shape title template=text
  fill $accent

component button variants=[primary, ghost] props=[label:text]
  frame root layout=flex(row, gap=8, padding=10,16, align=center, justify=between)
    fill $accent
    round-corners $radius.sm
    place icon shape=title at=0,0 size=10x10

frame card layout=flex(col, gap=12, padding=16) size=320x200 at=0,0
  fill $surface
  round-corners $radius.md
  place title shape=title at=0,0 size=200x20

instance cta from=button variant=primary label=\"Get started\" at=20,160
";
        let scene = dsl_parse::parse_file(clean).expect("c8 doc parses");
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output)
            .unwrap_or_else(|e| panic!("emitted DSL failed to parse: {e}\n{output}"));
        assert_eq!(
            scene, scene2,
            "C8 round-trip mismatch\n--- DSL ---\n{output}"
        );

        // Spot-check the parsed model so a regression in either direction shows.
        assert_eq!(scene.design_tokens.len(), 4);
        assert_eq!(scene.components.len(), 1);
        assert_eq!(scene.components[0].variants, vec!["primary", "ghost"]);
        assert_eq!(
            scene.components[0].props,
            vec![("label".to_string(), "text".to_string())]
        );
        match &scene.nodes[0] {
            SceneNode::Frame(f) => {
                assert_eq!(f.name, "card");
                assert!(matches!(f.layout, Layout::Flex { .. }));
            }
            _ => panic!("expected frame"),
        }
        match scene.nodes.last() {
            Some(SceneNode::Instance(i)) => {
                assert_eq!(i.component, "button");
                assert_eq!(i.variant.as_deref(), Some("primary"));
                assert_eq!(i.props, vec![("label".into(), "Get started".into())]);
            }
            _ => panic!("expected instance"),
        }
    }

    /// Pre-C8 documents must round-trip byte-identically (no spurious `tokens`/
    /// empty blocks slipped in).
    #[test]
    fn round_trip_pre_c8_unchanged() {
        let input = "\
documentsize 400x400

palette
  bg #faf6f0

shape s template=rectangle
  fill $bg

place s shape=s at=0,0 size=400x400
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        assert!(
            !output.contains("\ntokens\n"),
            "pre-C8 scene must not emit a tokens block:\n{output}"
        );
        assert!(
            !output.contains("\ncomponent "),
            "pre-C8 scene must not emit a component block:\n{output}"
        );
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(scene, scene2);
    }

    /// All flex layout knobs survive the round trip and the padding compaction
    /// (1 / x,y / t,r,b,l) re-parses to the same edges.
    #[test]
    fn round_trip_layout_variants() {
        for layout in [
            "layout=none",
            "layout=flow",
            "layout=flex(row)",
            "layout=flex(col, gap=8)",
            "layout=flex(row, gap=4, padding=2,3,4,5)",
            "layout=flex(col, padding=7)",
            "layout=grid(columns=3)",
            "layout=grid(columns=2, gap=10)",
        ] {
            let input = format!("documentsize 100x100\n\nframe f {layout}\n");
            let scene = dsl_parse::parse_file(&input)
                .unwrap_or_else(|e| panic!("{layout} failed to parse: {e}"));
            let output = emit_scene(&scene);
            let scene2 = dsl_parse::parse_file(&output)
                .unwrap_or_else(|e| panic!("{layout} re-emit failed: {e}\n{output}"));
            assert_eq!(scene, scene2, "layout round-trip mismatch for {layout}");
        }
    }

    /// The design-system example round-trips byte-for-semantics:
    /// `parse(emit(scene)) == scene`.
    #[test]
    fn round_trip_design_system() {
        let input = include_str!("../../examples/design-system.strok");
        let scene = dsl_parse::parse_file(input).expect("design-system example parses");
        let output = emit_scene(&scene);
        let scene2 = dsl_parse::parse_file(&output)
            .unwrap_or_else(|e| panic!("design-system re-emit failed to parse: {e}\n{output}"));
        assert_eq!(scene, scene2, "design-system round-trip mismatch");
        // It exercises the full P4 surface.
        assert_eq!(scene.components.len(), 2, "expected button + navitem");
        assert!(scene.shapes.iter().any(|s| s.is_text()), "has text shape");
        assert!(!scene.design_tokens.is_empty(), "has generalized tokens");
    }

    #[test]
    fn round_trip_live_boolean_preserves_editable_operands() {
        let input = "\
documentsize 100x100

shape block template=rectangle

boolean silhouette op=union
  place head shape=block at=10,10 size=30x30
  place neck shape=block at=25,30 size=20x40 rotation=8
  fill #f7f3ea
  stroke none
";
        let scene = dsl_parse::parse_file(input).unwrap();
        let output = emit_scene(&scene);
        assert!(output.contains("boolean silhouette op=union"), "{output}");
        assert!(output.contains("place head shape=block"), "{output}");
        assert!(output.contains("place neck shape=block"), "{output}");
        let scene2 = dsl_parse::parse_file(&output).unwrap();
        assert_eq!(scene, scene2);
    }
}
