//! `UiDoc` — the framework-neutral UI intermediate representation.
//!
//! This is the single seam every framework backend renders. The invariant
//! that makes Strøk's "co-equal targets, no golden reference" promise real:
//!
//! > **Nothing in this module may be React-shaped or Solid-shaped.**
//!
//! There is no `className`, no `class`, no JSX fragment, no signal vs. hook.
//! Style is symbolic (tokens, not baked hex where a token exists). Layout is
//! expressed as intent (flex/grid/absolute), not as one framework's CSS-in-JS
//! dialect. A backend that needs a concept not present here must push that
//! concept *up* into the IR so every other backend gets it too — never
//! special-case it downstream. The first time a field exists "just for React,"
//! the abstraction has started to rot.
//!
//! Today the lowering (`crate::lower`) only produces the geometry subset
//! (one inline-SVG leaf + tokens), because the UX-primitive DSL (`frame` /
//! `component` / `layout=`, design doc §5) is not yet in the parser. The IR is
//! defined to its full target shape now so backends are written against the
//! final surface, not a throwaway one.

/// A complete, framework-neutral UI document — one emittable component.
#[derive(Debug, Clone, PartialEq)]
pub struct UiDoc {
    /// Component name, e.g. `"Button"`. Backends derive their identifier from this.
    pub name: String,
    /// Design tokens, kept symbolic. Shared verbatim with the `tailwind` target.
    pub tokens: TokenSet,
    /// The root node of the UI tree.
    pub root: UiNode,
    /// Notes produced during lowering (e.g. "effect rasterized — CSS can't express it").
    /// Surfaced to callers rather than silently discarded.
    pub diagnostics: Vec<String>,
}

/// Design tokens. Generalizes Strøk's `palette`/`$token` system to every
/// category a design system needs (C8 / E4.1): color/space/radius/font/shadow/
/// motion. `colors` is kept as a distinct ordered list (the historical surface,
/// and what the inline-SVG path resolves against); `generalized` carries the
/// full categorized set the `tailwind`/DTCG targets emit.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TokenSet {
    /// `(name, hex)` pairs, in declaration order (determinism).
    pub colors: Vec<(String, String)>,
    /// Every design token as `(category, name, value)`, in declaration order.
    /// Includes the colors above (under category `color`) plus non-color
    /// categories. The single view code/token targets consume.
    pub generalized: Vec<(String, String, String)>,
}

/// A node in the neutral UI tree.
#[derive(Debug, Clone, PartialEq)]
pub enum UiNode {
    /// A container element with layout + style and children.
    Element {
        tag: Tag,
        layout: Layout,
        style: Style,
        children: Vec<UiNode>,
    },
    /// A text leaf.
    Text(String),
    /// A vector leaf: raw SVG markup, inlined verbatim by every backend.
    Svg(String),
    /// A raster leaf: a reference to an exported asset.
    Image { src: String, alt: String },
    /// An instance of another component, with props.
    Instance {
        component: String,
        props: Vec<(String, String)>,
        children: Vec<UiNode>,
    },
}

/// Semantic element tag. Neutral — backends map this to their own element syntax.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Tag {
    Div,
    Section,
    Span,
    Button,
}

impl Tag {
    pub fn html_name(self) -> &'static str {
        match self {
            Tag::Div => "div",
            Tag::Section => "section",
            Tag::Span => "span",
            Tag::Button => "button",
        }
    }
}

/// Layout intent. Lowers to flex/grid/absolute utility classes.
#[derive(Debug, Clone, PartialEq)]
pub enum Layout {
    /// No layout opinion (normal flow).
    Flow,
    Flex {
        direction: FlexDirection,
        gap: f64,
        padding: Edges,
        align: Align,
        justify: Justify,
    },
    Grid {
        columns: u32,
        gap: f64,
    },
    /// Absolutely positioned at (x, y) with explicit size.
    Absolute {
        x: f64,
        y: f64,
        w: f64,
        h: f64,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FlexDirection {
    Row,
    Col,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Align {
    Start,
    Center,
    End,
    Stretch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Justify {
    Start,
    Center,
    End,
    Between,
}

/// Per-edge insets in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Default)]
pub struct Edges {
    pub top: f64,
    pub right: f64,
    pub bottom: f64,
    pub left: f64,
}

impl Edges {
    pub fn uniform(v: f64) -> Self {
        Edges {
            top: v,
            right: v,
            bottom: v,
            left: v,
        }
    }
    pub fn symmetric(x: f64, y: f64) -> Self {
        Edges {
            top: y,
            right: x,
            bottom: y,
            left: x,
        }
    }
    fn is_zero(&self) -> bool {
        self.top == 0.0 && self.right == 0.0 && self.bottom == 0.0 && self.left == 0.0
    }
}

/// Visual style. Symbolic where possible: a `StyleValue::Token` stays a token
/// (→ `bg-<token>`) so the design system, not a hex literal, is the source.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Style {
    pub fill: Option<StyleValue>,
    pub radius: Option<f64>,
    pub opacity: Option<f64>,
    pub size: Option<(f64, f64)>,
    /// Typographic style, present on text leaves (C9 / E4.3). Lowers to
    /// `text-*`/`font-*` utility classes and a CSS `color`, so text is real,
    /// selectable, accessible DOM text — not rasterized inside an SVG. Boxed so
    /// the common (non-text) `Style` stays small and `UiNode` doesn't bloat.
    pub text: Option<Box<TextStyle>>,
}

/// Typographic style for a text leaf (C9 / E4.3). Symbolic where a token
/// exists: a `font.body` token stays `font-body`, a `$ink` color stays
/// `text-ink`, so the design system remains the source.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TextStyle {
    /// Font family — a token (`font-body`) or a raw family name.
    pub family: Option<StyleValue>,
    /// Font size in px.
    pub size: Option<f64>,
    /// Font weight (e.g. `bold`, `600`).
    pub weight: Option<String>,
    /// Text color — a token (`text-ink`) or a raw color.
    pub color: Option<StyleValue>,
    /// Text alignment (from the shape's `text-anchor`).
    pub align: Option<TextAlign>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAlign {
    Left,
    Center,
    Right,
}

/// A style value: a literal, or a symbolic design-token reference.
#[derive(Debug, Clone, PartialEq)]
pub enum StyleValue {
    Raw(String),
    Token(String),
}

// ── Utility-class lowering ──────────────────────────────────────────────
//
// Style/Layout → Tailwind-style utility classes. This lives in the neutral
// layer (not in any backend) precisely so React and Solid produce identical
// class strings. Backends differ only in *where* the class list is attached
// (`className` vs `class`), never in *what* it contains. Class order is fixed
// for deterministic, diffable output.

impl StyleValue {
    fn bg_class(&self) -> String {
        match self {
            StyleValue::Token(t) => format!("bg-{t}"),
            StyleValue::Raw(v) => format!("bg-[{v}]"),
        }
    }
    fn text_color_class(&self) -> String {
        match self {
            StyleValue::Token(t) => format!("text-{t}"),
            StyleValue::Raw(v) => format!("text-[{v}]"),
        }
    }
    fn font_family_class(&self) -> String {
        match self {
            StyleValue::Token(t) => format!("font-{t}"),
            StyleValue::Raw(v) => format!("font-[{v}]"),
        }
    }
}

impl TextStyle {
    /// Typographic utility classes, fixed order for diffable output.
    pub fn utility_classes(&self) -> Vec<String> {
        let mut c = Vec::new();
        if let Some(f) = &self.family {
            c.push(f.font_family_class());
        }
        if let Some(s) = self.size {
            c.push(format!("text-[{}]", px(s)));
        }
        if let Some(w) = &self.weight {
            c.push(format!("font-[{w}]"));
        }
        if let Some(col) = &self.color {
            c.push(col.text_color_class());
        }
        if let Some(a) = self.align {
            c.push(match a {
                TextAlign::Left => "text-left".into(),
                TextAlign::Center => "text-center".into(),
                TextAlign::Right => "text-right".into(),
            });
        }
        c
    }
}

/// Render a number as a compact px length (`12`, not `12.0`).
fn px(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("{}px", v as i64)
    } else {
        format!("{v}px")
    }
}

impl Layout {
    pub fn utility_classes(&self) -> Vec<String> {
        let mut c = Vec::new();
        match self {
            Layout::Flow => {}
            Layout::Flex {
                direction,
                gap,
                padding,
                align,
                justify,
            } => {
                c.push("flex".into());
                c.push(match direction {
                    FlexDirection::Row => "flex-row".into(),
                    FlexDirection::Col => "flex-col".into(),
                });
                if *gap != 0.0 {
                    c.push(format!("gap-[{}]", px(*gap)));
                }
                c.extend(padding_classes(padding));
                c.push(match align {
                    Align::Start => "items-start".into(),
                    Align::Center => "items-center".into(),
                    Align::End => "items-end".into(),
                    Align::Stretch => "items-stretch".into(),
                });
                c.push(match justify {
                    Justify::Start => "justify-start".into(),
                    Justify::Center => "justify-center".into(),
                    Justify::End => "justify-end".into(),
                    Justify::Between => "justify-between".into(),
                });
            }
            Layout::Grid { columns, gap } => {
                c.push("grid".into());
                c.push(format!("grid-cols-{columns}"));
                if *gap != 0.0 {
                    c.push(format!("gap-[{}]", px(*gap)));
                }
            }
            Layout::Absolute { x, y, w, h } => {
                c.push("absolute".into());
                c.push(format!("left-[{}]", px(*x)));
                c.push(format!("top-[{}]", px(*y)));
                c.push(format!("w-[{}]", px(*w)));
                c.push(format!("h-[{}]", px(*h)));
            }
        }
        c
    }
}

fn padding_classes(p: &Edges) -> Vec<String> {
    if p.is_zero() {
        return Vec::new();
    }
    if p.top == p.right && p.right == p.bottom && p.bottom == p.left {
        return vec![format!("p-[{}]", px(p.top))];
    }
    if p.top == p.bottom && p.left == p.right {
        return vec![
            format!("px-[{}]", px(p.left)),
            format!("py-[{}]", px(p.top)),
        ];
    }
    vec![
        format!("pt-[{}]", px(p.top)),
        format!("pr-[{}]", px(p.right)),
        format!("pb-[{}]", px(p.bottom)),
        format!("pl-[{}]", px(p.left)),
    ]
}

impl Style {
    pub fn utility_classes(&self) -> Vec<String> {
        let mut c = Vec::new();
        if let Some((w, h)) = self.size {
            c.push(format!("w-[{}]", px(w)));
            c.push(format!("h-[{}]", px(h)));
        }
        if let Some(fill) = &self.fill {
            c.push(fill.bg_class());
        }
        if let Some(r) = self.radius {
            c.push(format!("rounded-[{}]", px(r)));
        }
        if let Some(o) = self.opacity {
            c.push(format!("opacity-{}", (o * 100.0).round() as i64));
        }
        if let Some(text) = &self.text {
            c.extend(text.utility_classes());
        }
        c
    }
}

impl UiNode {
    /// The full, ordered class list for an element node (layout then style).
    /// Returns `None` for leaves that don't carry a class list.
    pub fn class_list(&self) -> Option<String> {
        match self {
            UiNode::Element { layout, style, .. } => {
                let mut classes = layout.utility_classes();
                classes.extend(style.utility_classes());
                Some(classes.join(" "))
            }
            _ => None,
        }
    }
}
