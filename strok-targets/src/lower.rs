//! `Scene → UiDoc` lowering — the one shared path all backends sit behind.
//!
//! C8 (E4.1/E4.2) made this the *real* UX-primitive lowering: `frame`s become
//! layout [`UiNode::Element`] containers, `place`s become per-shape inline-SVG
//! leaves, `instance`s become [`UiNode::Instance`] nodes, generalized `tokens`
//! become a [`TokenSet`] across all categories, and each `component` lowers to
//! its own [`UiDoc`] (one emitted file). A scene that uses none of these
//! constructs still lowers to a single inline-SVG leaf, exactly as before — so
//! nothing regresses. Backends and the cross-backend parity test are untouched:
//! they consume the same `UiNode` surface they always did, which is the proof
//! the IR seam holds.

use strok_core::resolve;
use strok_core::scene::{
    Component, FlexAlign, FlexDirection, FlexJustify, Frame, Instance, Layout as SceneLayout,
    Place, RadiusValue, Scene, SceneNode,
};
use strok_core::types::Color;

use crate::ir::{
    Align, Edges, FlexDirection as IrFlexDir, Justify, Layout, Style, StyleValue, Tag, TextAlign,
    TextStyle, TokenSet, UiDoc, UiNode,
};
use crate::target::{EmitOptions, Result};

/// Lower a scene's top-level content to the neutral UI IR (the main document).
///
/// Frames/instances produce a structured tree; a scene with only geometry
/// (shapes + places, no frame/instance) falls back to the historical
/// single-inline-SVG leaf so every pre-C8 example lowers byte-identically.
pub fn lower_scene(scene: &Scene, opts: &EmitOptions) -> Result<UiDoc> {
    // The structural tree (frame styles, tokens, instance props) is lowered from
    // the *unresolved* scene so design tokens stay symbolic (`bg-surface`, not a
    // baked hex). Inline-SVG leaves use the *resolved* scene so a browser paints
    // concrete colors. Two scenes, one lowering — the seam still holds.
    let resolved = resolve::apply_scheme(scene, opts.scheme.as_deref())?;
    let ctx = LowerCtx {
        scene,
        resolved: &resolved,
    };
    let tokens = lower_tokens(scene);

    let name = opts
        .component_name
        .clone()
        .unwrap_or_else(|| "StrokComponent".to_string());

    let mut diagnostics = Vec::new();

    // Does the scene use any UX primitive at the top level? If not, keep the
    // legacy whole-scene-as-one-SVG behavior (and its diagnostic).
    let uses_ux = scene
        .nodes
        .iter()
        .any(|n| matches!(n, SceneNode::Frame(_) | SceneNode::Instance(_)));

    let root = if uses_ux {
        let children: Vec<UiNode> = scene
            .nodes
            .iter()
            .filter_map(|n| lower_node(&ctx, n))
            .collect();
        UiNode::Element {
            tag: Tag::Div,
            layout: Layout::Flow,
            style: Style {
                size: Some((scene.document_size.w, scene.document_size.h)),
                ..Style::default()
            },
            children,
        }
    } else {
        let svg = resolve::resolve_scene(&resolved);
        diagnostics.push(
            "scene has no frames/instances; lowered as a single inline-SVG leaf \
             (geometry-only document)."
                .to_string(),
        );
        UiNode::Element {
            tag: Tag::Div,
            layout: Layout::Flow,
            style: Style {
                size: Some((resolved.document_size.w, resolved.document_size.h)),
                ..Style::default()
            },
            children: vec![UiNode::Svg(svg)],
        }
    };

    Ok(UiDoc {
        name,
        tokens,
        root,
        diagnostics,
    })
}

/// Shared lowering context: the original (symbolic) scene plus the
/// scheme-resolved scene used for concrete inline-SVG colors.
struct LowerCtx<'a> {
    scene: &'a Scene,
    resolved: &'a Scene,
}

/// Lower every `component` definition to its own [`UiDoc`] (one emitted file
/// each). Variants and props are surfaced in the doc's diagnostics for now;
/// the structural tree is variant-neutral (variant-scoped overrides are a
/// follow-up — recorded honestly, not hidden).
pub fn lower_components(scene: &Scene, opts: &EmitOptions) -> Result<Vec<UiDoc>> {
    let resolved = resolve::apply_scheme(scene, opts.scheme.as_deref())?;
    let ctx = LowerCtx {
        scene,
        resolved: &resolved,
    };
    let tokens = lower_tokens(scene);
    let mut docs = Vec::new();
    for component in &scene.components {
        docs.push(lower_component(&ctx, component, &tokens));
    }
    Ok(docs)
}

fn lower_component(ctx: &LowerCtx, component: &Component, tokens: &TokenSet) -> UiDoc {
    let children: Vec<UiNode> = component
        .children
        .iter()
        .filter_map(|n| lower_node(ctx, n))
        .collect();

    let root = if children.len() == 1 {
        // A single child (the usual `frame root`) is the component root directly.
        children.into_iter().next().unwrap_or(UiNode::Element {
            tag: Tag::Div,
            layout: Layout::Flow,
            style: Style::default(),
            children: vec![],
        })
    } else {
        UiNode::Element {
            tag: Tag::Div,
            layout: Layout::Flow,
            style: Style::default(),
            children,
        }
    };

    let mut diagnostics = Vec::new();
    if !component.variants.is_empty() {
        diagnostics.push(format!(
            "component '{}' declares variants [{}]; variant-scoped style overrides \
             are not yet lowered — emitted structure is variant-neutral.",
            component.name,
            component.variants.join(", ")
        ));
    }

    UiDoc {
        name: component.name.clone(),
        tokens: tokens.clone(),
        root,
        diagnostics,
    }
}

/// Lower a single scene node. Returns `None` for nodes with no UI projection
/// (e.g. links, which are resolved through place references).
fn lower_node(ctx: &LowerCtx, node: &SceneNode) -> Option<UiNode> {
    match node {
        SceneNode::Frame(frame) => Some(lower_frame(ctx, frame)),
        SceneNode::Instance(inst) => Some(lower_instance(inst)),
        SceneNode::Place(place) => Some(lower_place(ctx, place)),
        SceneNode::Group(group) => {
            // A group is a transform-only geometry grouping; render its subtree
            // as an inline SVG leaf so the vector content is preserved.
            Some(lower_group(ctx, &group.name))
        }
        SceneNode::Boolean(boolean) => {
            // A live boolean is resolved to its cohesive path before it reaches
            // UI targets, while its editable operands remain in the source.
            Some(lower_group(ctx, &boolean.name))
        }
        SceneNode::Link(_) => None,
    }
}

/// Lower a `frame` to a styled layout container with lowered children.
fn lower_frame(ctx: &LowerCtx, frame: &Frame) -> UiNode {
    let children: Vec<UiNode> = frame
        .children
        .iter()
        .filter_map(|n| lower_node(ctx, n))
        .collect();

    UiNode::Element {
        tag: Tag::Div,
        layout: lower_layout(&frame.layout),
        style: lower_frame_style(ctx, frame),
        children,
    }
}

fn lower_frame_style(ctx: &LowerCtx, frame: &Frame) -> Style {
    Style {
        fill: frame.fill.as_ref().and_then(lower_fill),
        radius: frame.radius.as_ref().map(|r| match r {
            RadiusValue::Literal(n) => *n,
            // A `$radius.md` token resolves to its numeric value from the token
            // set so the CSS `rounded-[Npx]` is real, not 0.
            RadiusValue::Token(t) => resolve_radius_token(ctx.scene, t),
        }),
        opacity: frame.opacity,
        size: frame.size.map(|d| (d.w, d.h)),
        text: None,
    }
}

/// Resolve a `$radius.md`-style token reference to its numeric value from the
/// scene's design tokens. Accepts dotted (`radius.md`) and bare (`md`) names;
/// falls back to `0.0` if undefined (the diagnostic-worthy case is rare and the
/// container simply renders square-cornered rather than panicking).
fn resolve_radius_token(scene: &Scene, token: &str) -> f64 {
    let want = token.strip_prefix("radius.").unwrap_or(token);
    for t in &scene.design_tokens {
        let matches = t.dotted() == token || (t.category == "radius" && t.name == want);
        if matches {
            if let Ok(n) = t.value.parse::<f64>() {
                return n;
            }
        }
    }
    0.0
}

/// Map a frame fill `Color` to a symbolic or raw style value. A `$token` stays
/// symbolic (`bg-<token>`); a concrete color becomes a raw value.
fn lower_fill(color: &Color) -> Option<StyleValue> {
    match color {
        Color::Token(t) => Some(StyleValue::Token(t.clone())),
        Color::None => None,
        Color::Hex(h) => Some(StyleValue::Raw(h.clone())),
        Color::CurrentColor => Some(StyleValue::Raw("currentColor".to_string())),
        // Gradients can't be a single bg token/value — leave the container
        // unfilled (the inline-SVG child still carries the gradient).
        Color::LinearGradient(_) | Color::RadialGradient(_) => None,
    }
}

fn lower_instance(inst: &Instance) -> UiNode {
    let mut props: Vec<(String, String)> = Vec::new();
    if let Some(v) = &inst.variant {
        props.push(("variant".to_string(), v.clone()));
    }
    for (k, val) in &inst.props {
        props.push((k.clone(), val.clone()));
    }
    UiNode::Instance {
        component: inst.component.clone(),
        props,
        children: vec![],
    }
}

/// Lower a single placed node. A `place` whose shape is `template=text` becomes
/// a **real, selectable, accessible** [`UiNode::Text`] leaf wrapped in a styled
/// `<span>` (C9 / E4.3) — not rasterized inside an SVG. Any other place becomes
/// an inline-SVG leaf carrying just that node's geometry, with concrete colors
/// from the resolved scene a browser can paint.
fn lower_place(ctx: &LowerCtx, place: &Place) -> UiNode {
    // Text-as-UI: lower text places to DOM text, not SVG. The geometry/color
    // come from the unresolved shape (tokens stay symbolic) but the inline-SVG
    // fallback uses the resolved scene.
    if let Some(shape) = ctx.scene.find_shape(&place.shape_ref) {
        if shape.is_text() {
            return lower_text_place(ctx, place, shape);
        }
    }
    // Vector leaf: resolve just this node. If it isn't reachable in
    // `scene.nodes` (e.g. a `place` that lives only inside a `component`), fall
    // back to a synthetic preview scene so component-internal geometry still
    // renders (C8 follow-up #2 — resolved here).
    let svg = resolve_place_svg(ctx, place);
    UiNode::Svg(svg)
}

/// Lower a `group` node to an inline-SVG leaf for its whole subtree.
fn lower_group(ctx: &LowerCtx, node_name: &str) -> UiNode {
    UiNode::Svg(resolve::resolve_scene_single_node(ctx.resolved, node_name))
}

/// Resolve a place to inline SVG. Prefers the in-scene single-node path; for a
/// component-internal place (absent from `scene.nodes`) it builds a synthetic
/// preview scene sized to the place, so the icon geometry resolves at its own
/// size rather than emitting an empty `<svg>` (the C8 follow-up #2, resolved).
fn resolve_place_svg(ctx: &LowerCtx, place: &Place) -> String {
    if ctx.resolved.find_node(&place.name).is_some() {
        return resolve::resolve_scene_single_node(ctx.resolved, &place.name);
    }
    // Component-internal place: synthesize a one-shape preview scene whose canvas
    // is the place's own size, so the leaf SVG is the icon, not the whole doc.
    if ctx.resolved.find_shape(&place.shape_ref).is_some() {
        let mut preview = ctx.resolved.clone();
        if let Some(size) = place.size {
            preview.document_size = size;
        }
        let mut leaf = place.clone();
        leaf.position = strok_core::scene::PlacePosition::At(0.0, 0.0);
        leaf.anchor = None;
        leaf.offset = None;
        preview.nodes = vec![strok_core::scene::SceneNode::Place(leaf)];
        return resolve::resolve_scene_single_node(&preview, &place.name);
    }
    // Last-resort empty canvas (shape genuinely missing) — surfaced, not hidden.
    resolve::resolve_scene_single_node(ctx.resolved, &place.name)
}

/// Lower a text `place` to a styled span carrying real DOM text. Typographic
/// style is symbolic where a token exists (`font-body`, `text-ink`).
fn lower_text_place(ctx: &LowerCtx, place: &Place, shape: &strok_core::shape::Shape) -> UiNode {
    let content = effective_text(place, shape).unwrap_or_default();

    let family = shape.font_family().map(style_value_from_str);
    let color = effective_fill(ctx, place, shape);
    let align = shape.text_anchor().map(map_text_anchor);

    let text = TextStyle {
        family,
        size: shape.font_size(),
        weight: shape.font_weight().map(|w| w.to_string()),
        color,
        align,
    };

    UiNode::Element {
        tag: Tag::Span,
        layout: Layout::Flow,
        style: Style {
            text: Some(Box::new(text)),
            ..Style::default()
        },
        children: vec![UiNode::Text(content)],
    }
}

/// The text content for a place: a place-level `content` override wins, else the
/// shape's content.
fn effective_text(place: &Place, shape: &strok_core::shape::Shape) -> Option<String> {
    for op in place.overrides.iter().rev() {
        if let strok_core::shape::Operation::Content(s) = op {
            return Some(s.clone());
        }
    }
    shape.content().map(|s| s.to_string())
}

/// The effective fill/color for a text place: a place-level `fill` override wins
/// (kept symbolic if a token), else the shape's fill.
fn effective_fill(
    _ctx: &LowerCtx,
    place: &Place,
    shape: &strok_core::shape::Shape,
) -> Option<StyleValue> {
    for op in place.overrides.iter().rev() {
        if let strok_core::shape::Operation::Fill(c) = op {
            return color_to_text_value(c);
        }
    }
    shape.fill().and_then(color_to_text_value)
}

fn color_to_text_value(color: &Color) -> Option<StyleValue> {
    match color {
        Color::Token(t) => Some(StyleValue::Token(t.clone())),
        Color::None => None,
        Color::Hex(h) => Some(StyleValue::Raw(h.clone())),
        Color::CurrentColor => Some(StyleValue::Raw("currentColor".to_string())),
        Color::LinearGradient(_) | Color::RadialGradient(_) => None,
    }
}

/// A font-family string: `$token` stays symbolic (the `font.` category prefix is
/// dropped so `$font.body` → the Tailwind `font-body` class against the
/// `--font-body` @theme var); anything else is raw (quotes stripped, spaces →
/// `_` since Tailwind arbitrary values don't take quotes/spaces).
fn style_value_from_str(s: &str) -> StyleValue {
    if let Some(tok) = s.strip_prefix('$') {
        let name = tok.strip_prefix("font.").unwrap_or(tok);
        StyleValue::Token(name.to_string())
    } else {
        StyleValue::Raw(s.trim_matches('"').replace(' ', "_"))
    }
}

fn map_text_anchor(a: strok_core::types::TextAnchor) -> TextAlign {
    use strok_core::types::TextAnchor as TA;
    match a {
        TA::Start => TextAlign::Left,
        TA::Middle => TextAlign::Center,
        TA::End => TextAlign::Right,
    }
}

/// Lower the scene's tokens (palette colors + generalized `tokens`) to a
/// [`TokenSet`]. `colors` keeps the historical ordered color list; `generalized`
/// carries every category for the tailwind/DTCG targets.
fn lower_tokens(scene: &Scene) -> TokenSet {
    let all = scene.all_tokens();
    TokenSet {
        colors: scene.palette.tokens.clone(),
        generalized: all
            .into_iter()
            .map(|t| (t.category, t.name, t.value))
            .collect(),
    }
}

/// Map the core DSL [`SceneLayout`] to the IR [`Layout`].
fn lower_layout(layout: &SceneLayout) -> Layout {
    match layout {
        SceneLayout::None => Layout::Flow,
        SceneLayout::Flow => Layout::Flow,
        SceneLayout::Flex {
            direction,
            gap,
            padding,
            align,
            justify,
        } => {
            let (t, r, b, l) = *padding;
            Layout::Flex {
                direction: match direction {
                    FlexDirection::Row => IrFlexDir::Row,
                    FlexDirection::Col => IrFlexDir::Col,
                },
                gap: *gap,
                padding: Edges {
                    top: t,
                    right: r,
                    bottom: b,
                    left: l,
                },
                align: match align {
                    FlexAlign::Start => Align::Start,
                    FlexAlign::Center => Align::Center,
                    FlexAlign::End => Align::End,
                    FlexAlign::Stretch => Align::Stretch,
                },
                justify: match justify {
                    FlexJustify::Start => Justify::Start,
                    FlexJustify::Center => Justify::Center,
                    FlexJustify::End => Justify::End,
                    FlexJustify::Between => Justify::Between,
                },
            }
        }
        SceneLayout::Grid { columns, gap } => Layout::Grid {
            columns: *columns,
            gap: *gap,
        },
    }
}
