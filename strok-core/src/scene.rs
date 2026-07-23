/// v3 scene graph — the top-level document model.
///
/// A Scene contains shape definitions and scene nodes (Place, Group, Link).
/// The .strok file IS the construction history: shapes define templates,
/// scene nodes compose them into the final image.
use crate::shape::{Effect, Operation, Shape};
use crate::types::*;

/// An import declaration: `use "./path.strok"` or `use "./path.strok" as namespace`.
#[derive(Debug, Clone, PartialEq)]
pub struct Import {
    pub path: String,
    pub namespace: Option<String>,
}

/// Named color tokens plus theme overrides.
///
/// `tokens` is the base palette (also the fallback for every scheme).
/// Each `ColorScheme` overrides a subset of tokens. Colors written `$name`
/// in the DSL resolve here at render time (see `resolve::apply_scheme`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Palette {
    pub tokens: Vec<(String, String)>,
    pub schemes: Vec<ColorScheme>,
}

/// A named theme: token overrides layered over the base palette.
#[derive(Debug, Clone, PartialEq)]
pub struct ColorScheme {
    pub name: String,
    pub tokens: Vec<(String, String)>,
}

impl Palette {
    pub fn is_empty(&self) -> bool {
        self.tokens.is_empty() && self.schemes.is_empty()
    }

    pub fn has_scheme(&self, name: &str) -> bool {
        self.schemes.iter().any(|s| s.name == name)
    }

    /// Resolve a token to a hex/color string, preferring the named scheme's
    /// override and falling back to the base palette.
    pub fn resolve<'a>(&'a self, token: &str, scheme: Option<&str>) -> Option<&'a str> {
        if let Some(name) = scheme {
            if let Some(sc) = self.schemes.iter().find(|s| s.name == name) {
                if let Some((_, hex)) = sc.tokens.iter().find(|(t, _)| t == token) {
                    return Some(hex.as_str());
                }
            }
        }
        self.tokens
            .iter()
            .find(|(t, _)| t == token)
            .map(|(_, h)| h.as_str())
    }
}

/// A generalized design token (C8 / E4.1). Extends the color-only `palette`
/// system to spacing, radius, type scale, shadows, motion — every category a
/// design system needs, all referenceable as `$category.name` in the DSL and all
/// flowing to the `tailwind` target's `@theme`.
///
/// `palette` colors are *also* design tokens — they're surfaced under the
/// `color` category (see [`Scene::all_tokens`]) so existing `$copper` references
/// keep resolving while `$color.copper` is the generalized spelling. This keeps
/// the DSL additive/backwards-compatible.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DesignToken {
    /// Category: `color`, `space`, `radius`, `font`, `shadow`, `motion`, … .
    pub category: String,
    /// Token name within the category (e.g. `md`, `accent`).
    pub name: String,
    /// The literal value. Numbers stay as written (`16`); strings keep quotes
    /// for round-trip (`"IBM Plex Sans"`); colors are hex.
    pub value: String,
}

impl DesignToken {
    /// The dotted reference spelling, `category.name` (the `$`-prefixed form a
    /// DSL value uses). Token-ref spelling is **dotted** (D-4 lean, settled in
    /// C8): `$radius.md`, not `$radius-md`.
    pub fn dotted(&self) -> String {
        format!("{}.{}", self.category, self.name)
    }
}

/// Layout policy for a [`Frame`] (C8 / E4.1). Maps 1:1 to the `strok-targets`
/// IR `Layout` enum, which maps to CSS flex/grid. Kept in `strok-core` as the
/// parsed DSL form so the model round-trips without depending on the targets crate.
#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    /// No layout opinion (`layout=none` / absent).
    None,
    /// Normal flow (`layout=flow`).
    Flow,
    /// Flexbox: `layout=flex(row|col, gap=N, padding=…, align=…, justify=…)`.
    Flex {
        direction: FlexDirection,
        gap: f64,
        /// Padding edges `(top, right, bottom, left)`.
        padding: (f64, f64, f64, f64),
        align: FlexAlign,
        justify: FlexJustify,
    },
    /// Grid: `layout=grid(columns=N, gap=N)`.
    Grid { columns: u32, gap: f64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    Col,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexAlign {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexJustify {
    Start,
    Center,
    End,
    Between,
}

/// A layout container (Figma frame / artboard) — C8 / E4.1. Groups children and
/// carries a layout policy; lowers to a styled container element (`<div>`),
/// distinct from `group` (which is a transform-only geometry grouping).
#[derive(Debug, Clone, PartialEq)]
pub struct Frame {
    pub name: String,
    pub layout: Layout,
    /// Explicit size `size=WxH`, if given.
    pub size: Option<Dimension>,
    /// Absolute position `at=x,y`, if given.
    pub position: Option<(f64, f64)>,
    /// Container fill (`fill $surface` / hex / token).
    pub fill: Option<Color>,
    /// Corner radius (`round-corners N` or `$radius.md`). Stored resolved to a
    /// number plus the optional token name it came from (for round-trip).
    pub radius: Option<RadiusValue>,
    /// Container opacity.
    pub opacity: Option<f64>,
    /// Children: places, nested frames, groups, instances.
    pub children: Vec<SceneNode>,
}

/// A radius value that may be a literal number or a `$token` reference, kept
/// distinct so the DSL round-trips the original spelling.
#[derive(Debug, Clone, PartialEq)]
pub enum RadiusValue {
    Literal(f64),
    Token(String),
}

/// A reusable, parameterizable UI subtree (C8 / E4.2). Lowers to its own
/// emitted component file; `instance` references it with prop/variant overrides.
#[derive(Debug, Clone, PartialEq)]
pub struct Component {
    pub name: String,
    /// Declared variant names (e.g. `[primary, ghost]`); first is the default.
    pub variants: Vec<String>,
    /// Declared props as `(name, type)` (e.g. `[label:text]`).
    pub props: Vec<(String, String)>,
    /// The component body: frames/places/instances.
    pub children: Vec<SceneNode>,
}

/// An instance of a [`Component`] (C8 / E4.2).
#[derive(Debug, Clone, PartialEq)]
pub struct Instance {
    pub name: String,
    /// The referenced component name (`from=button`).
    pub component: String,
    /// Selected variant (`variant=primary`), if any.
    pub variant: Option<String>,
    /// Prop overrides as `(name, value)`, in declaration order.
    pub props: Vec<(String, String)>,
    /// Absolute position `at=x,y`, if any.
    pub position: Option<(f64, f64)>,
    /// Explicit size `size=WxH`, if any.
    pub size: Option<Dimension>,
}

/// Top-level scene container.
#[derive(Debug, Clone, PartialEq)]
pub struct Scene {
    pub document_size: Dimension,
    pub imports: Vec<Import>,
    pub palette: Palette,
    /// Generalized design tokens beyond color (C8 / E4.1): space/radius/font/
    /// shadow/motion. `palette` colors are surfaced alongside these via
    /// [`Scene::all_tokens`]. Empty for pre-C8 scenes (byte-identical round-trip).
    pub design_tokens: Vec<DesignToken>,
    /// `let` bindings (C13): `(name, original expr source, evaluated value)`, in
    /// declaration order. Each is a scalar `f64` referenceable as `$name` in
    /// expressions. The source string is kept verbatim so `emit` re-emits the
    /// original `let <name> <expr>` and `parse(emit(scene)) == scene` holds.
    pub lets: Vec<(String, String, f64)>,
    pub defaults: Vec<Operation>,
    pub shapes: Vec<Shape>,
    /// Reusable component definitions (C8 / E4.2).
    pub components: Vec<Component>,
    pub nodes: Vec<SceneNode>,
    /// Names of shapes that were merged in by `resolve_imports` (from a
    /// filesystem `use` or an embedded `std/` module), as opposed to being
    /// defined locally in this document (EXP-1). `emit_scene` must skip these
    /// when re-emitting `scene.shapes` — imports are re-emitted as `use`
    /// lines, not by inlining every shape they define, or every re-save of a
    /// document that imports a module (std or local) would duplicate the
    /// whole module's shapes into the file and grow unboundedly on repeated
    /// read-modify-write.
    pub imported_shape_names: std::collections::BTreeSet<String>,
}

impl Scene {
    pub fn new(size: Dimension) -> Self {
        Scene {
            document_size: size,
            imports: Vec::new(),
            palette: Palette::default(),
            design_tokens: Vec::new(),
            lets: Vec::new(),
            defaults: Vec::new(),
            shapes: Vec::new(),
            components: Vec::new(),
            nodes: Vec::new(),
            imported_shape_names: std::collections::BTreeSet::new(),
        }
    }

    /// Find a component definition by name.
    pub fn find_component(&self, name: &str) -> Option<&Component> {
        self.components.iter().find(|c| c.name == name)
    }

    /// Every design token, with `palette` colors surfaced under the `color`
    /// category first (so `$copper` and `$color.copper` resolve to the same
    /// value), followed by the explicit `tokens` block entries in order. This is
    /// the unified view the `tailwind`/code targets consume.
    pub fn all_tokens(&self) -> Vec<DesignToken> {
        let mut out = Vec::new();
        for (name, value) in &self.palette.tokens {
            out.push(DesignToken {
                category: "color".to_string(),
                name: name.clone(),
                value: value.clone(),
            });
        }
        out.extend(self.design_tokens.iter().cloned());
        out
    }

    /// Find a shape definition by name.
    pub fn find_shape(&self, name: &str) -> Option<&Shape> {
        self.shapes.iter().find(|s| s.name == name)
    }

    /// Find a placed node by name (searches top-level and within groups).
    pub fn find_node(&self, name: &str) -> Option<&SceneNode> {
        find_node_in(&self.nodes, name)
    }

    /// Build a synthetic preview scene that places a shape at the origin so it
    /// can be inspected or rendered like any other scene node.
    pub fn make_shape_preview(&self, shape_name: &str) -> Option<(Self, String)> {
        self.find_shape(shape_name)?;

        let mut preview_scene = self.clone();
        let preview_name = self.unique_preview_name(shape_name);
        preview_scene.nodes.push(SceneNode::Place(Place {
            name: preview_name.clone(),
            shape_ref: shape_name.to_string(),
            position: PlacePosition::At(0.0, 0.0),
            size: None,
            rotation: None,
            flip: None,
            skew: None,
            clip: None,
            mask: None,
            anchor: None,
            overrides: Vec::new(),
            align: None,
            offset: None,
            text_path: None,
        }));

        Some((preview_scene, preview_name))
    }

    fn unique_preview_name(&self, shape_name: &str) -> String {
        let base = format!("__preview_{}__", shape_name);
        if self.find_node(&base).is_none() {
            return base;
        }

        // `usize::MAX` candidates is astronomically more than any real scene
        // could have nodes, so this loop always returns. We use a bounded range
        // (not `1..`) and a deterministic fallback instead of `unreachable!()`
        // so the no-panic policy holds without an `#[allow]` (C1 handoff item).
        for i in 1..=usize::MAX {
            let candidate = format!("{}{}", base, i);
            if self.find_node(&candidate).is_none() {
                return candidate;
            }
        }
        // Unreachable in practice; return a UUID-free deterministic last resort.
        format!("{}{}", base, usize::MAX)
    }
}

fn find_node_in<'a>(nodes: &'a [SceneNode], name: &str) -> Option<&'a SceneNode> {
    for node in nodes {
        match node {
            SceneNode::Place(p) if p.name == name => return Some(node),
            SceneNode::Group(g) => {
                if g.name == name {
                    return Some(node);
                }
                if let Some(found) = find_node_in(&g.children, name) {
                    return Some(found);
                }
            }
            SceneNode::Link(l) if l.name == name => return Some(node),
            SceneNode::Frame(fr) => {
                if fr.name == name {
                    return Some(node);
                }
                if let Some(found) = find_node_in(&fr.children, name) {
                    return Some(found);
                }
            }
            SceneNode::Instance(i) if i.name == name => return Some(node),
            _ => {}
        }
    }
    None
}

/// A node in the scene tree.
#[derive(Debug, Clone, PartialEq)]
pub enum SceneNode {
    Place(Place),
    Group(Group),
    Link(Link),
    /// A layout container (C8 / E4.1).
    Frame(Frame),
    /// A component instance (C8 / E4.2).
    Instance(Instance),
}

/// Relative placement anchor.
#[derive(Debug, Clone, PartialEq)]
pub enum PlaceAnchor {
    Above { target: String, gap: f64 },
    Below { target: String, gap: f64 },
}

/// Anchor point on a bounding box (9-point grid).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BboxAnchor {
    TopLeft,
    Top,
    TopRight,
    Left,
    Center,
    Right,
    BottomLeft,
    Bottom,
    BottomRight,
}

impl BboxAnchor {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "tl" => Some(Self::TopLeft),
            "top" => Some(Self::Top),
            "tr" => Some(Self::TopRight),
            "left" => Some(Self::Left),
            "center" => Some(Self::Center),
            "right" => Some(Self::Right),
            "bl" => Some(Self::BottomLeft),
            "bottom" => Some(Self::Bottom),
            "br" => Some(Self::BottomRight),
            _ => None,
        }
    }

    /// Returns (fx, fy) where 0..1 represents fraction across the bbox.
    pub fn factors(&self) -> (f64, f64) {
        match self {
            Self::TopLeft => (0.0, 0.0),
            Self::Top => (0.5, 0.0),
            Self::TopRight => (1.0, 0.0),
            Self::Left => (0.0, 0.5),
            Self::Center => (0.5, 0.5),
            Self::Right => (1.0, 0.5),
            Self::BottomLeft => (0.0, 1.0),
            Self::Bottom => (0.5, 1.0),
            Self::BottomRight => (1.0, 1.0),
        }
    }
}

impl std::fmt::Display for BboxAnchor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::TopLeft => write!(f, "tl"),
            Self::Top => write!(f, "top"),
            Self::TopRight => write!(f, "tr"),
            Self::Left => write!(f, "left"),
            Self::Center => write!(f, "center"),
            Self::Right => write!(f, "right"),
            Self::BottomLeft => write!(f, "bl"),
            Self::Bottom => write!(f, "bottom"),
            Self::BottomRight => write!(f, "br"),
        }
    }
}

/// A shape instance placed in the scene.
#[derive(Debug, Clone, PartialEq)]
pub struct Place {
    pub name: String,
    pub shape_ref: String,
    pub position: PlacePosition,
    pub size: Option<Dimension>,
    pub rotation: Option<Rotation>,
    pub flip: Option<Flip>,
    /// Skew in degrees `(x, y)` (E2.3). Applied about the place's center, like
    /// rotation. `None` ⇒ no skew (pre-C4 places stay byte-identical).
    pub skew: Option<(f64, f64)>,
    /// Per-place clip: clip this shape to the geometry of the named shape(s)
    /// (E2.4). Multiple names ⇒ clip by their union.
    pub clip: Option<Vec<String>>,
    /// Per-place alpha/luminance mask (E2.4): the named shape's luminance gates
    /// this shape's alpha.
    pub mask: Option<String>,
    /// Relative placement anchor (above=/below= with optional gap=).
    pub anchor: Option<PlaceAnchor>,
    /// Inline attribute overrides (e.g. `fill #b83050` after place line).
    pub overrides: Vec<Operation>,
    /// Which point on THIS shape goes at the target point (default: tl).
    pub align: Option<BboxAnchor>,
    /// Nudge from the resolved position.
    pub offset: Option<(f64, f64)>,
    /// Render a text shape along the geometry of another placed shape (E2.7).
    /// Holds the *place name* of the path to flow along; only meaningful when
    /// `shape_ref` resolves to a `text` shape. Emits `<textPath href="#id">`.
    /// `None` ⇒ ordinary `<text>` (pre-C5 places stay byte-identical).
    pub text_path: Option<String>,
}

/// How a shape is positioned in the scene.
#[derive(Debug, Clone, PartialEq)]
pub enum PlacePosition {
    /// Absolute coordinate: `at=x,y`.
    At(f64, f64),
    /// Parametric on a path: `on=shape.point at=percent`.
    On {
        path: PointRef,
        t: RelativeSize,
        side: Option<Side>,
        offset: Option<AbsoluteSize>,
    },
    /// Relative to another placed element's bounding box anchor.
    RelativeTo { target: String, anchor: BboxAnchor },
}

/// A group of scene nodes.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub name: String,
    pub children: Vec<SceneNode>,
    /// Group-level position offset (translate).
    pub position: Option<(f64, f64)>,
    /// Group-level rotation in degrees.
    pub rotation: Option<Rotation>,
    /// Group-level flip (mirror).
    pub flip: Option<Flip>,
    /// Group-level skew in degrees `(x, y)` (E2.3).
    pub skew: Option<(f64, f64)>,
    /// Clip children to the geometry of the named shape(s). Multiple names ⇒
    /// clip by their union (E2.4: clip-by-multiple-shapes).
    pub clip: Option<Vec<String>>,
    /// Alpha/luminance mask: the named shape's luminance gates the group's
    /// alpha (E2.4), distinct from the hard-edged `clip`.
    pub mask: Option<String>,
    /// Group-level opacity (applied to all children as a unit).
    pub opacity: Option<f64>,
}

/// A linked instance that inherits from a source shape.
#[derive(Debug, Clone, PartialEq)]
pub struct Link {
    pub name: String,
    pub source: String,
    /// Override operations (typically just attribute overrides).
    pub overrides: Vec<Operation>,
    /// Override effects.
    pub effects: Vec<Effect>,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::shape::Template;

    #[test]
    fn scene_find_shape() {
        let mut scene = Scene::new(Dimension { w: 400.0, h: 400.0 });
        scene.shapes.push(Shape::new("bg", Template::Rectangle));
        assert!(scene.find_shape("bg").is_some());
        assert!(scene.find_shape("nope").is_none());
    }

    #[test]
    fn scene_find_node_in_group() {
        let scene = Scene {
            document_size: Dimension { w: 400.0, h: 400.0 },
            imports: Vec::new(),
            palette: Palette::default(),
            design_tokens: Vec::new(),
            lets: Vec::new(),
            defaults: Vec::new(),
            shapes: Vec::new(),
            components: Vec::new(),
            imported_shape_names: Default::default(),
            nodes: vec![SceneNode::Group(Group {
                name: "bloom".to_string(),
                children: vec![SceneNode::Place(Place {
                    name: "petal-1".to_string(),
                    shape_ref: "petal".to_string(),
                    position: PlacePosition::At(100.0, 100.0),
                    size: None,
                    rotation: None,
                    flip: None,
                    skew: None,
                    clip: None,
                    mask: None,
                    anchor: None,
                    overrides: Vec::new(),
                    align: None,
                    offset: None,
                    text_path: None,
                })],
                position: None,
                rotation: None,
                flip: None,
                skew: None,
                clip: None,
                mask: None,
                opacity: None,
            })],
        };
        assert!(scene.find_node("petal-1").is_some());
        assert!(scene.find_node("bloom").is_some());
    }

    #[test]
    fn bbox_anchor_parse_roundtrip() {
        let names = [
            "tl", "top", "tr", "left", "center", "right", "bl", "bottom", "br",
        ];
        for name in &names {
            let anchor =
                BboxAnchor::parse(name).unwrap_or_else(|| panic!("failed to parse '{}'", name));
            assert_eq!(&format!("{}", anchor), *name);
        }
    }

    #[test]
    fn bbox_anchor_parse_invalid() {
        assert!(BboxAnchor::parse("bogus").is_none());
        assert!(BboxAnchor::parse("").is_none());
        assert!(BboxAnchor::parse("CENTER").is_none());
    }

    #[test]
    fn bbox_anchor_factors() {
        assert_eq!(BboxAnchor::TopLeft.factors(), (0.0, 0.0));
        assert_eq!(BboxAnchor::Center.factors(), (0.5, 0.5));
        assert_eq!(BboxAnchor::BottomRight.factors(), (1.0, 1.0));
        assert_eq!(BboxAnchor::Top.factors(), (0.5, 0.0));
        assert_eq!(BboxAnchor::Right.factors(), (1.0, 0.5));
        assert_eq!(BboxAnchor::Bottom.factors(), (0.5, 1.0));
        assert_eq!(BboxAnchor::Left.factors(), (0.0, 0.5));
        assert_eq!(BboxAnchor::TopRight.factors(), (1.0, 0.0));
        assert_eq!(BboxAnchor::BottomLeft.factors(), (0.0, 1.0));
    }

    #[test]
    fn make_shape_preview_adds_synthetic_place() {
        let mut scene = Scene::new(Dimension { w: 400.0, h: 400.0 });
        scene.shapes.push(Shape::new("badge", Template::Rectangle));

        let (preview_scene, preview_name) = scene.make_shape_preview("badge").unwrap();

        assert_eq!(preview_name, "__preview_badge__");
        let preview_node = preview_scene.find_node(&preview_name);
        assert!(preview_node.is_some());
        match preview_node.unwrap() {
            SceneNode::Place(place) => {
                assert_eq!(place.shape_ref, "badge");
                assert_eq!(place.position, PlacePosition::At(0.0, 0.0));
            }
            _ => panic!("expected preview node to be a place"),
        }
    }

    #[test]
    fn make_shape_preview_avoids_existing_node_names() {
        let mut scene = Scene::new(Dimension { w: 400.0, h: 400.0 });
        scene.shapes.push(Shape::new("badge", Template::Rectangle));
        scene.nodes.push(SceneNode::Place(Place {
            name: "__preview_badge__".to_string(),
            shape_ref: "badge".to_string(),
            position: PlacePosition::At(10.0, 10.0),
            size: None,
            rotation: None,
            flip: None,
            skew: None,
            clip: None,
            mask: None,
            anchor: None,
            overrides: Vec::new(),
            align: None,
            offset: None,
            text_path: None,
        }));

        let (_, preview_name) = scene.make_shape_preview("badge").unwrap();
        assert_eq!(preview_name, "__preview_badge__1");
    }

    #[test]
    fn make_shape_preview_requires_existing_shape() {
        let scene = Scene::new(Dimension { w: 400.0, h: 400.0 });
        assert!(scene.make_shape_preview("missing").is_none());
    }
}
