/// v3 shape model — templates sculpted by operations.
///
/// A Shape is a named template plus a sequence of operations that sculpt it.
/// `resolve()` replays operations on the template's default geometry to produce PathData.
use crate::path_point::{CurveMode, NamedPoint, PathData};
use crate::types::*;

// ── Templates ─────────────────────────────────────────────────────────

/// Base geometric template. Defines initial points and topology.
#[derive(Debug, Clone, PartialEq)]
pub enum Template {
    Rectangle,
    Ellipse,
    Triangle,
    Line,
    Path,
    Text,
}

impl Template {
    pub fn parse(s: &str) -> crate::error::Result<Self> {
        match s {
            "rectangle" => Ok(Template::Rectangle),
            "ellipse" => Ok(Template::Ellipse),
            "triangle" => Ok(Template::Triangle),
            "line" => Ok(Template::Line),
            "path" => Ok(Template::Path),
            "text" => Ok(Template::Text),
            _ => Err(crate::error::StrokError::ParseError(format!(
                "unknown template '{}' — valid templates: rectangle, ellipse, triangle, line, path, text",
                s
            ))),
        }
    }
}

impl std::fmt::Display for Template {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Template::Rectangle => write!(f, "rectangle"),
            Template::Ellipse => write!(f, "ellipse"),
            Template::Triangle => write!(f, "triangle"),
            Template::Line => write!(f, "line"),
            Template::Path => write!(f, "path"),
            Template::Text => write!(f, "text"),
        }
    }
}

// ── Operations ────────────────────────────────────────────────────────

/// An operation that mutates a shape during construction.
#[derive(Debug, Clone, PartialEq)]
pub enum Operation {
    // Point position mutations
    MovePointDelta {
        point: String,
        dx: f64,
        dy: f64,
    },
    MovePointTo {
        point: String,
        to: (f64, f64),
    },
    PullPointDir {
        point: String,
        dir: Direction,
        amount: RelativeSize,
    },
    PullPointDelta {
        point: String,
        dx: f64,
        dy: f64,
        radius: usize,
        falloff: NormalizedAmount,
    },
    Sculpt {
        at: SculptTarget,
        dx: f64,
        dy: f64,
        radius: Option<AbsoluteSize>,
        falloff: Option<NormalizedAmount>,
        axis: Option<SculptAxis>,
        lock_endpoints: bool,
    },

    // Line mutations (topology)
    AddPoint {
        name: String,
        at: (f64, f64),
        after: Option<String>,
        mode: Option<PointMode>,
        tension: Option<f64>,
        arc_rx: Option<f64>,
        arc_ry: Option<f64>,
        arc_sweep: Option<bool>,
        arc_large: Option<bool>,
        /// Order-independent bulge side for `mode=arc`. When set it overrides
        /// `arc_sweep`; when unset, `arc_sweep` (raw SVG flag) is used.
        arc_bulge: Option<ArcBulge>,
        control_c1: Option<(f64, f64)>,
        control_c2: Option<(f64, f64)>,
    },
    SplitLine {
        segment: SegmentRef,
        name: String,
        t: Option<NormalizedAmount>,
    },
    DeletePoint {
        point: String,
        reconnect: Option<ReconnectMode>,
    },

    // Shape mutations
    Close,
    Open,
    /// Begin a new subpath at the *next* added point. Emitted by the C3 boolean /
    /// stroke-outline / offset ops to carry holes and disjoint contours in one
    /// `path` shape. During `resolve` it records that the following `AddPoint`
    /// starts a fresh contour (its index lands in `PathData::subpath_starts`).
    /// A no-op if no point follows it. Round-trips as the DSL line `subpath`.
    Subpath,
    Smooth {
        point: String,
        tension: Option<f64>,
    },
    /// Smooth both segments adjacent to an anchor. Point modes are stored on
    /// incoming segments, so this sets the named point and the next point in
    /// the same contour to Catmull-Rom with one shared tension.
    SmoothCorner {
        point: String,
        tension: Option<f64>,
    },
    SmoothAll {
        tension: Option<f64>,
    },
    Sharpen {
        point: String,
    },
    SharpenAll,
    /// Convert a point's curve mode in place, preserving its position (E2.5).
    /// `to` selects the target anchor type. Unlike `smooth`/`sharpen` (which only
    /// reach catmull/sharp) this also reaches `arc` and explicit `controls`,
    /// deriving handles from the neighbouring points so the conversion is a
    /// no-surprise default the author can then nudge. Round-trips + has an inverse
    /// (convert back to the prior mode).
    ConvertPoint {
        point: String,
        to: ConvertTarget,
    },
    RoundCorners {
        /// Per-corner radii (E2.6). A uniform `round-corners 8` becomes
        /// `CornerRadii::uniform(8)`; `round-corners tl=8 tr=8 br=0 bl=0`
        /// addresses each corner by point name. Round-trips both spellings.
        radii: CornerRadii,
    },
    /// Cut a rectangular notch INTO an edge, or push a tab OUT from it (E2.6).
    /// First-class so authors stop hand-composing a rounded-rect + a separate
    /// sharp path for folder tabs, speech-bubble tails, and corner-folds.
    /// `dir` selects inward (notch) vs outward (tail); `shape` selects a square
    /// cut vs a triangular point. Operates on a rectangle edge or a path edge.
    Notch(NotchSpec),

    // Text
    Content(String),
    FontSize(f64),
    FontFamily(String),
    FontWeight(String),
    FontStyle(String),

    // Attributes
    Fill(Color),
    FillRule(FillRule),
    Stroke(Color),
    StrokeWidth(AbsoluteSize),
    StrokeLinecap(LineCap),
    StrokeLinejoin(LineJoin),
    StrokeMiterlimit(f64),
    StrokeDasharray(Vec<f64>),
    Opacity(NormalizedAmount),
    Blur(f64),
    TextAnchor(TextAnchor),
}

/// Target anchor type for `convert-point` (E2.5).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConvertTarget {
    /// Hard corner (line in/out).
    Sharp,
    /// Catmull-Rom smooth (tension 0).
    Smooth,
    /// Elliptical arc segment.
    Arc,
    /// Explicit cubic bezier control handles.
    Controls,
}

impl ConvertTarget {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "sharp" => Some(Self::Sharp),
            "smooth" => Some(Self::Smooth),
            "arc" => Some(Self::Arc),
            "controls" => Some(Self::Controls),
            _ => None,
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Sharp => "sharp",
            Self::Smooth => "smooth",
            Self::Arc => "arc",
            Self::Controls => "controls",
        }
    }
}

/// Per-corner radius spec for `round-corners` (E2.6).
///
/// `Uniform` keeps the pre-C5 single-radius behaviour (every corner the same)
/// and round-trips as `round-corners <r>`. `PerCorner` carries an optional
/// radius for each corner *by point name* (rectangles use `tl`/`tr`/`br`/`bl`),
/// round-tripping as `round-corners tl=8 tr=8 br=0 bl=0`. A corner not listed in
/// `PerCorner` is left sharp (radius 0).
#[derive(Debug, Clone, PartialEq)]
pub enum CornerRadii {
    /// One radius for every corner.
    Uniform(f64),
    /// Radius per named corner; absent ⇒ 0 (sharp). Order-preserving so emit is
    /// deterministic.
    PerCorner(Vec<(String, f64)>),
}

impl CornerRadii {
    pub fn uniform(r: f64) -> Self {
        CornerRadii::Uniform(r)
    }

    /// Radius for the corner with this point name.
    pub fn radius_for(&self, name: &str) -> f64 {
        match self {
            CornerRadii::Uniform(r) => *r,
            CornerRadii::PerCorner(list) => list
                .iter()
                .find(|(n, _)| n == name)
                .map(|(_, r)| *r)
                .unwrap_or(0.0),
        }
    }
}

/// Which direction a notch/tail goes relative to the edge it sits on (E2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotchDir {
    /// Cut inward (a bite out of the shape) — e.g. a tab slot, a corner-fold.
    In,
    /// Push outward (a protrusion) — e.g. a folder tab, a speech-bubble tail.
    Out,
}

impl NotchDir {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "in" | "notch" => Some(Self::In),
            "out" | "tail" | "tab" => Some(Self::Out),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::In => "in",
            Self::Out => "out",
        }
    }
}

/// Square cut vs triangular point (E2.6).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NotchShape {
    /// Rectangular step (folder tab, slot).
    Square,
    /// Triangular point (speech-bubble tail, arrow).
    Triangle,
}

impl NotchShape {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "square" | "rect" => Some(Self::Square),
            "triangle" | "point" => Some(Self::Triangle),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Square => "square",
            Self::Triangle => "triangle",
        }
    }
}

/// Which edge of a rectangle (or, for paths, the segment between two named
/// points) a notch/tail sits on.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NotchEdge {
    /// A named rectangle edge.
    Named(Edge),
    /// The segment from the first point name to the second (for `path` shapes).
    Segment(String, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Edge {
    Top,
    Bottom,
    Left,
    Right,
}

impl Edge {
    pub fn parse(s: &str) -> Option<Self> {
        match s {
            "top" => Some(Self::Top),
            "bottom" => Some(Self::Bottom),
            "left" => Some(Self::Left),
            "right" => Some(Self::Right),
            _ => None,
        }
    }
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::Top => "top",
            Self::Bottom => "bottom",
            Self::Left => "left",
            Self::Right => "right",
        }
    }
}

/// A notch/tail primitive (E2.6). See `Operation::Notch`.
#[derive(Debug, Clone, PartialEq)]
pub struct NotchSpec {
    pub edge: NotchEdge,
    pub dir: NotchDir,
    pub shape: NotchShape,
    /// Center of the notch along the edge, 0..1 from the edge's start point.
    pub pos: f64,
    /// Width of the notch along the edge, in document units.
    pub width: f64,
    /// Depth perpendicular to the edge, in document units.
    pub depth: f64,
}

/// Target for sculpt operations.
#[derive(Debug, Clone, PartialEq)]
pub enum SculptTarget {
    Point(PointRef),
    Coord(f64, f64),
    Segment(SegmentRef),
}

// ── Effects ───────────────────────────────────────────────────────────

/// Non-destructive, render-time effect.
#[derive(Debug, Clone, PartialEq)]
pub enum Effect {
    Droop {
        amount: NormalizedAmount,
        direction: Option<Direction>,
    },
    Curl {
        amount: NormalizedAmount,
        from: Option<PointRef>,
    },
    Taper {
        start: RelativeSize,
        end: RelativeSize,
    },
    Jitter {
        amount: NormalizedAmount,
        seed: Option<u32>,
    },
}

// ── Shape ─────────────────────────────────────────────────────────────

/// A shape definition: a template sculpted by operations.
#[derive(Debug, Clone, PartialEq)]
pub struct Shape {
    pub name: String,
    pub template: Template,
    pub operations: Vec<Operation>,
    pub effects: Vec<Effect>,
}

impl Shape {
    /// Create a new shape from a template.
    pub fn new(name: &str, template: Template) -> Self {
        Shape {
            name: name.to_string(),
            template,
            operations: Vec::new(),
            effects: Vec::new(),
        }
    }

    /// Get the fill color from operations (last one wins).
    pub fn fill(&self) -> Option<&Color> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::Fill(c) = op {
                Some(c)
            } else {
                None
            }
        })
    }

    /// Get the stroke color from operations (last one wins).
    pub fn stroke(&self) -> Option<&Color> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::Stroke(c) = op {
                Some(c)
            } else {
                None
            }
        })
    }

    /// Get stroke width from operations (last one wins).
    pub fn stroke_width(&self) -> Option<f64> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::StrokeWidth(w) = op {
                Some(w.0)
            } else {
                None
            }
        })
    }

    /// Get stroke linecap from operations (last one wins).
    pub fn stroke_linecap(&self) -> Option<LineCap> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::StrokeLinecap(c) = op {
                Some(*c)
            } else {
                None
            }
        })
    }

    /// Get stroke linejoin from operations (last one wins).
    pub fn stroke_linejoin(&self) -> Option<LineJoin> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::StrokeLinejoin(j) = op {
                Some(*j)
            } else {
                None
            }
        })
    }

    /// Get stroke miterlimit from operations (last one wins).
    pub fn stroke_miterlimit(&self) -> Option<f64> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::StrokeMiterlimit(m) = op {
                Some(*m)
            } else {
                None
            }
        })
    }

    /// Get fill-rule from operations (last one wins).
    pub fn fill_rule(&self) -> Option<FillRule> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::FillRule(r) = op {
                Some(*r)
            } else {
                None
            }
        })
    }

    /// Get stroke dasharray from operations (last one wins).
    pub fn stroke_dasharray(&self) -> Option<&[f64]> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::StrokeDasharray(v) = op {
                Some(v.as_slice())
            } else {
                None
            }
        })
    }

    /// Get text content from operations.
    pub fn content(&self) -> Option<&str> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::Content(s) = op {
                Some(s.as_str())
            } else {
                None
            }
        })
    }

    /// Get font-size from operations (last one wins).
    pub fn font_size(&self) -> Option<f64> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::FontSize(v) = op {
                Some(*v)
            } else {
                None
            }
        })
    }

    /// Get font-family from operations (last one wins).
    pub fn font_family(&self) -> Option<&str> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::FontFamily(s) = op {
                Some(s.as_str())
            } else {
                None
            }
        })
    }

    /// Get font-weight from operations (last one wins).
    pub fn font_weight(&self) -> Option<&str> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::FontWeight(s) = op {
                Some(s.as_str())
            } else {
                None
            }
        })
    }

    /// Get font-style from operations (last one wins).
    pub fn font_style(&self) -> Option<&str> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::FontStyle(s) = op {
                Some(s.as_str())
            } else {
                None
            }
        })
    }

    /// Get text-anchor from operations (last one wins).
    pub fn text_anchor(&self) -> Option<TextAnchor> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::TextAnchor(a) = op {
                Some(*a)
            } else {
                None
            }
        })
    }

    /// Whether this shape is a text shape.
    pub fn is_text(&self) -> bool {
        self.template == Template::Text
    }

    /// Get opacity from operations (last one wins).
    pub fn opacity(&self) -> Option<f64> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::Opacity(a) = op {
                Some(a.0)
            } else {
                None
            }
        })
    }

    /// Get blur radius from operations (last one wins).
    pub fn blur(&self) -> Option<f64> {
        self.operations.iter().rev().find_map(|op| {
            if let Operation::Blur(r) = op {
                Some(*r)
            } else {
                None
            }
        })
    }

    /// Resolve this shape into PathData by replaying operations on the template.
    ///
    /// The `coord_space` is the size used for the path's local coordinate system.
    /// For templates other than `Path`, we use a 100x100 normalized space.
    pub fn resolve(&self, coord_space: (f64, f64)) -> PathData {
        self.resolve_scaled(coord_space, None)
    }

    /// Like [`Shape::resolve`], but `radius_scale = (sx, sy)` tells geometry ops
    /// whose parameters are lengths in *placed* space (currently `round-corners`)
    /// what bbox-fit scale the placement will apply, so they can compensate.
    /// A `round-corners 8` then measures 8 units in the placed element, not
    /// `8 × size/authored-bbox`. `None` ⇒ no compensation (authored space).
    pub fn resolve_scaled(
        &self,
        coord_space: (f64, f64),
        radius_scale: Option<(f64, f64)>,
    ) -> PathData {
        let mut pd = self.initial_geometry(coord_space);
        // When a `Subpath` op was just seen, the next AddPoint begins a new
        // contour; we record its index in `pd.subpath_starts`.
        let mut pending_subpath = false;

        for op in &self.operations {
            match op {
                Operation::MovePointDelta { point, dx, dy } => {
                    if let Some(p) = pd.points.iter_mut().find(|p| p.name == *point) {
                        p.x += dx;
                        p.y += dy;
                    }
                }
                Operation::MovePointTo { point, to } => {
                    if let Some(p) = pd.points.iter_mut().find(|p| p.name == *point) {
                        p.x = to.0;
                        p.y = to.1;
                    }
                }
                Operation::PullPointDir { point, dir, amount } => {
                    if let Some(p) = pd.points.iter_mut().find(|p| p.name == *point) {
                        let frac = amount.0 / 100.0;
                        match dir {
                            Direction::Up => p.y -= coord_space.1 * frac,
                            Direction::Down => p.y += coord_space.1 * frac,
                            Direction::Left => p.x -= coord_space.0 * frac,
                            Direction::Right => p.x += coord_space.0 * frac,
                        }
                    }
                }
                Operation::PullPointDelta {
                    point,
                    dx,
                    dy,
                    radius,
                    falloff,
                } => {
                    if let Some(center) = pd.points.iter().position(|p| p.name == *point) {
                        let n = pd.points.len();
                        let rad = *radius;
                        let fo = falloff.0;
                        for i in 0..n {
                            let linear = i.abs_diff(center);
                            let dist = if pd.closed {
                                linear.min(n - linear)
                            } else {
                                linear
                            };
                            if dist > rad {
                                continue;
                            }
                            let weight = if rad == 0 {
                                1.0
                            } else {
                                let base = (rad + 1 - dist) as f64 / (rad + 1) as f64;
                                base.powf(fo)
                            };
                            pd.points[i].x += dx * weight;
                            pd.points[i].y += dy * weight;
                        }
                    }
                }
                Operation::Sculpt {
                    at,
                    dx,
                    dy,
                    radius,
                    falloff,
                    ..
                } => {
                    let (ax, ay) = match at {
                        SculptTarget::Coord(x, y) => (*x, *y),
                        SculptTarget::Point(pr) => {
                            if let Some(p) = pd.points.iter().find(|p| p.name == pr.point) {
                                (p.x, p.y)
                            } else {
                                continue;
                            }
                        }
                        SculptTarget::Segment(_) => continue, // TODO
                    };
                    let r = radius.map(|r| r.0).unwrap_or(50.0);
                    let fo = falloff.map(|f| f.0).unwrap_or(1.0);
                    for p in &mut pd.points {
                        let ddx = p.x - ax;
                        let ddy = p.y - ay;
                        let dist = (ddx * ddx + ddy * ddy).sqrt();
                        if dist > r {
                            continue;
                        }
                        let weight = (1.0 - dist / r).max(0.0).powf(fo);
                        p.x += dx * weight;
                        p.y += dy * weight;
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
                    let curve_mode = match mode {
                        Some(PointMode::CatmullRom) => {
                            CurveMode::CatmullRom(tension.unwrap_or(0.0))
                        }
                        Some(PointMode::Arc) => {
                            let rx = arc_rx.unwrap_or(10.0);
                            // Locate the point this arc departs from (the segment
                            // start) so a `bulge=` side can be resolved relative to
                            // the direction of travel. Insertion target is `after`
                            // when given, else the current last point.
                            let prev = if let Some(after_name) = after {
                                pd.points.iter().find(|p| p.name == *after_name)
                            } else {
                                pd.points.last()
                            };
                            // `bulge` (order-independent) takes precedence over the
                            // raw `sweep` flag. With neither set, behavior is
                            // byte-identical to before: sweep defaults to true.
                            let sweep = match arc_bulge {
                                Some(b) => {
                                    let (dx, dy) = match prev {
                                        Some(p) => (at.0 - p.x, at.1 - p.y),
                                        None => (0.0, 0.0),
                                    };
                                    b.to_sweep_flag(dx, dy)
                                }
                                None => arc_sweep.unwrap_or(true),
                            };
                            CurveMode::Arc {
                                rx,
                                ry: arc_ry.unwrap_or(rx),
                                sweep,
                                large: arc_large.unwrap_or(false),
                            }
                        }
                        Some(PointMode::Controls) => match (control_c1, control_c2) {
                            (Some(c1), Some(c2)) => CurveMode::Controls { c1: *c1, c2: *c2 },
                            _ => CurveMode::Sharp,
                        },
                        Some(PointMode::ControlsRelative) => match (control_c1, control_c2) {
                            (Some(c1), Some(c2)) => {
                                CurveMode::ControlsRelative { c1: *c1, c2: *c2 }
                            }
                            _ => CurveMode::Sharp,
                        },
                        _ => CurveMode::Sharp,
                    };
                    let point = NamedPoint {
                        name: name.clone(),
                        x: at.0,
                        y: at.1,
                        mode: curve_mode,
                    };
                    // A pending `Subpath` marks the new point as a contour start.
                    // Subpath geometry always appends (an `after=` mid-contour
                    // insert would scramble the contour index), so we honor the
                    // break only on the append path.
                    if let Some(after_name) = after {
                        if let Some(idx) = pd.points.iter().position(|p| p.name == *after_name) {
                            pd.points.insert(idx + 1, point);
                        } else {
                            if pending_subpath {
                                pd.subpath_starts.push(pd.points.len());
                            }
                            pd.points.push(point);
                        }
                    } else {
                        if pending_subpath {
                            pd.subpath_starts.push(pd.points.len());
                        }
                        pd.points.push(point);
                    }
                    pending_subpath = false;
                }
                Operation::Subpath => {
                    pending_subpath = true;
                }
                Operation::SplitLine { segment, name, t } => {
                    let t_val = t.map(|t| t.0).unwrap_or(0.5);
                    if let Some(from_idx) = pd.points.iter().position(|p| p.name == segment.p1) {
                        if let Some(to_idx) = pd.points.iter().position(|p| p.name == segment.p2) {
                            let from = &pd.points[from_idx];
                            let to = &pd.points[to_idx];
                            let x = from.x + (to.x - from.x) * t_val;
                            let y = from.y + (to.y - from.y) * t_val;
                            let insert_idx = if to_idx > from_idx {
                                to_idx
                            } else {
                                pd.points.len()
                            };
                            pd.points.insert(
                                insert_idx,
                                NamedPoint {
                                    name: name.clone(),
                                    x,
                                    y,
                                    mode: CurveMode::Sharp,
                                },
                            );
                        }
                    }
                }
                Operation::DeletePoint { point, .. } => {
                    pd.points.retain(|p| p.name != *point);
                }
                Operation::Close => {
                    pd.closed = true;
                }
                Operation::Open => {
                    pd.closed = false;
                }
                Operation::Smooth { point, tension } => {
                    if let Some(p) = pd.points.iter_mut().find(|p| p.name == *point) {
                        p.mode = CurveMode::CatmullRom(tension.unwrap_or(0.0));
                    }
                }
                Operation::SmoothCorner { point, tension } => {
                    if let Some(idx) = pd.points.iter().position(|p| p.name == *point) {
                        let t = tension.unwrap_or(0.0);
                        let next = next_point_in_contour(&pd, idx);
                        pd.points[idx].mode = CurveMode::CatmullRom(t);
                        if let Some(next_idx) = next {
                            pd.points[next_idx].mode = CurveMode::CatmullRom(t);
                        }
                    }
                }
                Operation::Sharpen { point } => {
                    if let Some(p) = pd.points.iter_mut().find(|p| p.name == *point) {
                        p.mode = CurveMode::Sharp;
                    }
                }
                Operation::SmoothAll { tension } => {
                    let t = tension.unwrap_or(0.0);
                    for p in pd.points.iter_mut() {
                        p.mode = CurveMode::CatmullRom(t);
                    }
                }
                Operation::SharpenAll => {
                    for p in pd.points.iter_mut() {
                        p.mode = CurveMode::Sharp;
                    }
                }
                Operation::ConvertPoint { point, to } => {
                    convert_point_mode(&mut pd, point, *to);
                }
                Operation::RoundCorners { radii } => {
                    // Any closed polygon with 3+ points can be filleted. This is
                    // especially important for triangle, shield, and logo marks;
                    // the old 4+ gate silently ignored round-corners on triangles.
                    if pd.points.len() >= 3 && pd.closed {
                        let n = pd.points.len();
                        let mut new_points = Vec::with_capacity(n * 2);
                        for i in 0..n {
                            let prev = &pd.points[(i + n - 1) % n];
                            let curr = &pd.points[i];
                            let next = &pd.points[(i + 1) % n];

                            // Per-corner radius (E2.6); 0 ⇒ leave the corner sharp.
                            let want = radii.radius_for(&curr.name);
                            if want <= 0.0 {
                                new_points.push(curr.clone());
                                continue;
                            }

                            // With a radius_scale, the radius is measured in
                            // placed space: edge lengths, the clamp, and the
                            // corner offsets are computed post-scale, and the
                            // authored arc is the ellipse that maps to a
                            // circular `r` under (sx, sy).
                            let (sx, sy) = radius_scale.unwrap_or((1.0, 1.0));

                            // Distance to prev and next, in placed space.
                            let dp = (((curr.x - prev.x) * sx).powi(2)
                                + ((curr.y - prev.y) * sy).powi(2))
                            .sqrt();
                            let dn = (((next.x - curr.x) * sx).powi(2)
                                + ((next.y - curr.y) * sy).powi(2))
                            .sqrt();

                            if dp < 1e-9 || dn < 1e-9 {
                                new_points.push(curr.clone());
                                continue;
                            }

                            // A true circular fillet of radius r touches each
                            // adjacent edge at distance r/tan(theta/2) from the
                            // corner, where theta is the interior angle. The old
                            // implementation used distance=r, which is only
                            // correct at 90° and produced visible bumps on shield,
                            // star, and logo corners.
                            let to_prev =
                                ((prev.x - curr.x) * sx / dp, (prev.y - curr.y) * sy / dp);
                            let to_next =
                                ((next.x - curr.x) * sx / dn, (next.y - curr.y) * sy / dn);
                            let dot =
                                (to_prev.0 * to_next.0 + to_prev.1 * to_next.1).clamp(-1.0, 1.0);
                            let angle = dot.acos();
                            if angle < 1e-6 || (std::f64::consts::PI - angle) < 1e-6 {
                                new_points.push(curr.clone());
                                continue;
                            }
                            // Keep exact right-angle corners byte-stable. Tiny
                            // floating error around tan(45°)=1 can otherwise
                            // make the downstream arc splitter see >90° and emit
                            // two 45° cubics instead of one quarter-circle cubic.
                            let tan_half = if (angle - std::f64::consts::FRAC_PI_2).abs() < 1e-12 {
                                1.0
                            } else {
                                (angle / 2.0).tan()
                            };
                            if !tan_half.is_finite() || tan_half.abs() < 1e-9 {
                                new_points.push(curr.clone());
                                continue;
                            }

                            // Clamp by tangent distance, then derive the actual
                            // radius. This preserves the requested radius until an
                            // adjacent edge is too short, at which point the fillet
                            // shrinks without overlapping the neighboring corner.
                            let offset = (want / tan_half).min(dp / 2.0).min(dn / 2.0);
                            let r = offset * tan_half;
                            if r < 1e-9 {
                                new_points.push(curr.clone());
                                continue;
                            }

                            // Offsets are fractions along the edges: a fraction
                            // is invariant under the placement's linear map, so
                            // computing it in placed space and applying it to
                            // authored coordinates lands the point correctly.
                            // Point approaching corner (along prev→curr edge, from curr side)
                            let t_in = offset / dp;
                            let in_x = curr.x + (prev.x - curr.x) * t_in;
                            let in_y = curr.y + (prev.y - curr.y) * t_in;

                            // Point leaving corner (along curr→next edge, from curr side)
                            let t_out = offset / dn;
                            let out_x = curr.x + (next.x - curr.x) * t_out;
                            let out_y = curr.y + (next.y - curr.y) * t_out;

                            new_points.push(NamedPoint {
                                name: format!("{}-in", curr.name),
                                x: in_x,
                                y: in_y,
                                mode: CurveMode::Sharp,
                            });
                            new_points.push(NamedPoint {
                                name: format!("{}-out", curr.name),
                                x: out_x,
                                y: out_y,
                                mode: CurveMode::Arc {
                                    // Authored-space ellipse that the placement
                                    // maps to a circular radius `r`.
                                    rx: r / sx.abs(),
                                    ry: r / sy.abs(),
                                    // Choose the turn direction from the placed
                                    // path orientation so clockwise and
                                    // counter-clockwise contours both fillet
                                    // toward their corner rather than flipping.
                                    sweep: ((curr.x - prev.x) * sx) * ((next.y - curr.y) * sy)
                                        - ((curr.y - prev.y) * sy) * ((next.x - curr.x) * sx)
                                        > 0.0,
                                    large: false,
                                },
                            });
                        }
                        pd.points = new_points;
                    }
                }
                Operation::Notch(spec) => {
                    // With a radius_scale the notch dimensions are measured in
                    // *placed* space (like round-corners): a `notch depth=20` on
                    // a bbox-fit `size=` element then measures 20 units on the
                    // final canvas, not `20 × size/authored-bbox` (EXP-5).
                    apply_notch(&mut pd, spec, radius_scale.unwrap_or((1.0, 1.0)));
                }
                // Attributes and text ops don't affect geometry
                Operation::Fill(_)
                | Operation::FillRule(_)
                | Operation::Stroke(_)
                | Operation::StrokeWidth(_)
                | Operation::StrokeLinecap(_)
                | Operation::StrokeLinejoin(_)
                | Operation::StrokeMiterlimit(_)
                | Operation::StrokeDasharray(_)
                | Operation::Opacity(_)
                | Operation::Blur(_)
                | Operation::TextAnchor(_)
                | Operation::Content(_)
                | Operation::FontSize(_)
                | Operation::FontFamily(_)
                | Operation::FontWeight(_)
                | Operation::FontStyle(_) => {}
            }
        }

        pd
    }

    /// Generate initial geometry for a template.
    fn initial_geometry(&self, coord_space: (f64, f64)) -> PathData {
        match self.template {
            Template::Rectangle => PathData {
                coord_space,
                points: vec![
                    NamedPoint {
                        name: "tl".to_string(),
                        x: 0.0,
                        y: 0.0,
                        mode: CurveMode::Sharp,
                    },
                    NamedPoint {
                        name: "tr".to_string(),
                        x: coord_space.0,
                        y: 0.0,
                        mode: CurveMode::Sharp,
                    },
                    NamedPoint {
                        name: "br".to_string(),
                        x: coord_space.0,
                        y: coord_space.1,
                        mode: CurveMode::Sharp,
                    },
                    NamedPoint {
                        name: "bl".to_string(),
                        x: 0.0,
                        y: coord_space.1,
                        mode: CurveMode::Sharp,
                    },
                ],
                closed: true,
                subpath_starts: Vec::new(),
            },
            Template::Ellipse => {
                let cx = coord_space.0 / 2.0;
                let cy = coord_space.1 / 2.0;
                // For a circle approximation with 4 cardinal points using our
                // Catmull-Rom formula (offset = (next-prev) * (1-tension)/6),
                // the correct tension is 1 - 4*(√2 - 1) = 5 - 4√2 ≈ -0.6569.
                //
                // Derivation: the standard Bézier circle constant is
                // κ = 4*(√2-1)/3 ≈ 0.5523. The control point must be at
                // distance r*κ from the anchor. Our formula uses the chord
                // between opposite neighbors (length 2r) scaled by (1-tension)/6,
                // so: r*κ = 2r * (1-tension)/6  →  tension = 1 - 3κ = 1 - 4*(√2-1).
                let tension = 1.0 - 4.0 * (std::f64::consts::SQRT_2 - 1.0);
                PathData {
                    coord_space,
                    points: vec![
                        NamedPoint {
                            name: "top".to_string(),
                            x: cx,
                            y: 0.0,
                            mode: CurveMode::CatmullRom(tension),
                        },
                        NamedPoint {
                            name: "right".to_string(),
                            x: coord_space.0,
                            y: cy,
                            mode: CurveMode::CatmullRom(tension),
                        },
                        NamedPoint {
                            name: "bottom".to_string(),
                            x: cx,
                            y: coord_space.1,
                            mode: CurveMode::CatmullRom(tension),
                        },
                        NamedPoint {
                            name: "left".to_string(),
                            x: 0.0,
                            y: cy,
                            mode: CurveMode::CatmullRom(tension),
                        },
                    ],
                    closed: true,
                    subpath_starts: Vec::new(),
                }
            }
            Template::Triangle => PathData {
                coord_space,
                points: vec![
                    NamedPoint {
                        name: "top".to_string(),
                        x: coord_space.0 / 2.0,
                        y: 0.0,
                        mode: CurveMode::Sharp,
                    },
                    NamedPoint {
                        name: "br".to_string(),
                        x: coord_space.0,
                        y: coord_space.1,
                        mode: CurveMode::Sharp,
                    },
                    NamedPoint {
                        name: "bl".to_string(),
                        x: 0.0,
                        y: coord_space.1,
                        mode: CurveMode::Sharp,
                    },
                ],
                closed: true,
                subpath_starts: Vec::new(),
            },
            Template::Line => PathData {
                coord_space,
                points: vec![
                    NamedPoint {
                        name: "start".to_string(),
                        x: 0.0,
                        y: 0.0,
                        mode: CurveMode::Sharp,
                    },
                    NamedPoint {
                        name: "end".to_string(),
                        x: coord_space.0,
                        y: coord_space.1,
                        mode: CurveMode::Sharp,
                    },
                ],
                closed: false,
                subpath_starts: Vec::new(),
            },
            Template::Path => PathData {
                coord_space,
                points: Vec::new(),
                closed: false,
                subpath_starts: Vec::new(),
            },
            Template::Text => PathData {
                coord_space,
                points: Vec::new(),
                closed: false,
                subpath_starts: Vec::new(),
            },
        }
    }
}

/// Convert the curve mode of the named point in place, preserving its position
/// (E2.5). For `arc`/`controls` the handles/radii are derived from the chord to
/// the previous point so the result is a sensible, editable default. Missing
/// point ⇒ no-op (no panic).
/// Map a rectangle edge to its (start, end) corner point names, walking the
/// path's winding order (tl → tr → br → bl). Returns `None` for non-rectangle
/// point sets.
fn rect_edge_points(edge: Edge) -> (&'static str, &'static str) {
    match edge {
        Edge::Top => ("tl", "tr"),
        Edge::Right => ("tr", "br"),
        Edge::Bottom => ("br", "bl"),
        Edge::Left => ("bl", "tl"),
    }
}

/// Apply a notch/tail primitive (E2.6): insert points between the two endpoints
/// of an edge so the edge gains a square step or triangular point, cut inward or
/// pushed outward. A no-op (never a panic) if the edge endpoints aren't found or
/// are coincident.
fn apply_notch(pd: &mut PathData, spec: &NotchSpec, scale: (f64, f64)) {
    let (start_name, end_name) = match &spec.edge {
        NotchEdge::Named(e) => {
            let (s, t) = rect_edge_points(*e);
            (s.to_string(), t.to_string())
        }
        NotchEdge::Segment(a, b) => (a.clone(), b.clone()),
    };

    let si = match pd.points.iter().position(|p| p.name == start_name) {
        Some(i) => i,
        None => return,
    };
    let ei = match pd.points.iter().position(|p| p.name == end_name) {
        Some(i) => i,
        None => return,
    };

    let (sx, sy) = (pd.points[si].x, pd.points[si].y);
    let (ex, ey) = (pd.points[ei].x, pd.points[ei].y);
    let edx = ex - sx;
    let edy = ey - sy;
    let len = (edx * edx + edy * edy).sqrt();
    if len <= f64::EPSILON {
        return;
    }
    // `width`, `depth` and the derived offsets are authored in *placed* space:
    // when a bbox-fit `size=` will later scale the shape by `scale = (scx, scy)`,
    // measure them on that final canvas so they don't silently shrink (EXP-5;
    // `scale = (1,1)` ⇒ the authored-space behaviour, byte-identical).
    let (scx, scy) = scale;
    // The placed edge vector and its length; widths/positions are fractions of
    // this so they land at the authored placed length. The perpendicular depth
    // is computed against the placed edge and mapped back through 1/scale.
    let (pedx, pedy) = (edx * scx, edy * scy);
    let plen = (pedx * pedx + pedy * pedy).sqrt();
    if plen <= f64::EPSILON {
        return;
    }
    // Unit placed tangent (start→end) and unit outward normal. A rectangle wound
    // tl→tr→br→bl is CLOCKWISE in screen coords (+y down), so its OUTWARD normal
    // is the tangent rotated −90°: (dy, −dx). E.g. the top edge (tangent +x)
    // points outward as (0, −1) = up. `In` flips it (a bite into the shape).
    let (ptx, pty) = (pedx / plen, pedy / plen);
    let (pnx, pny) = match spec.dir {
        NotchDir::Out => (pty, -ptx),
        NotchDir::In => (-pty, ptx),
    };

    let half_w = (spec.width / 2.0).min(plen / 2.0).max(0.0);
    let center = spec.pos.clamp(0.0, 1.0) * plen;
    let a = (center - half_w).clamp(0.0, plen);
    let b = (center + half_w).clamp(0.0, plen);
    let depth = spec.depth.max(0.0);

    // Fractions along the edge are invariant under the placement's linear map, so
    // computing them in placed space and applying to authored endpoints lands the
    // points correctly (same technique as `round-corners`).
    let (fa, fb, fc) = (a / plen, b / plen, center / plen);
    let pa = (sx + edx * fa, sy + edy * fa);
    let pb = (sx + edx * fb, sy + edy * fb);
    // Authored-space depth displacement that maps to `depth` in placed space:
    // displace by the placed normal, then divide each component by its scale.
    let (dx, dy) = (pnx * depth / scx, pny * depth / scy);

    let mut inserted: Vec<NamedPoint> = Vec::new();
    let dir_tag = spec.dir.as_str();
    match spec.shape {
        NotchShape::Square => {
            // a → out/in by depth → across → back to b.
            inserted.push(NamedPoint {
                name: format!("{}-{}-a", end_name, dir_tag),
                x: pa.0,
                y: pa.1,
                mode: CurveMode::Sharp,
            });
            inserted.push(NamedPoint {
                name: format!("{}-{}-a2", end_name, dir_tag),
                x: pa.0 + dx,
                y: pa.1 + dy,
                mode: CurveMode::Sharp,
            });
            inserted.push(NamedPoint {
                name: format!("{}-{}-b2", end_name, dir_tag),
                x: pb.0 + dx,
                y: pb.1 + dy,
                mode: CurveMode::Sharp,
            });
            inserted.push(NamedPoint {
                name: format!("{}-{}-b", end_name, dir_tag),
                x: pb.0,
                y: pb.1,
                mode: CurveMode::Sharp,
            });
        }
        NotchShape::Triangle => {
            // a → apex (at edge center, displaced by depth) → b.
            let mid = (sx + edx * fc, sy + edy * fc);
            inserted.push(NamedPoint {
                name: format!("{}-{}-a", end_name, dir_tag),
                x: pa.0,
                y: pa.1,
                mode: CurveMode::Sharp,
            });
            inserted.push(NamedPoint {
                name: format!("{}-{}-tip", end_name, dir_tag),
                x: mid.0 + dx,
                y: mid.1 + dy,
                mode: CurveMode::Sharp,
            });
            inserted.push(NamedPoint {
                name: format!("{}-{}-b", end_name, dir_tag),
                x: pb.0,
                y: pb.1,
                mode: CurveMode::Sharp,
            });
        }
    }

    // Insert after the start point. For the wrap-around edge (e.g. Left = bl→tl
    // where tl precedes bl in storage) si may be the last index, which still
    // means "right after the start point" along the winding.
    let insert_at = si + 1;
    if insert_at >= pd.points.len() {
        pd.points.extend(inserted);
    } else {
        for (k, p) in inserted.into_iter().enumerate() {
            pd.points.insert(insert_at + k, p);
        }
    }
}

fn convert_point_mode(pd: &mut PathData, point: &str, to: ConvertTarget) {
    let idx = match pd.points.iter().position(|p| p.name == point) {
        Some(i) => i,
        None => return,
    };
    // The segment ARRIVING at `idx` departs from the previous point (or, for a
    // closed path, the last point when idx==0).
    let n = pd.points.len();
    let prev_idx = if idx > 0 {
        Some(idx - 1)
    } else if pd.closed && n > 1 {
        Some(n - 1)
    } else {
        None
    };
    let (px, py) = prev_idx
        .map(|i| (pd.points[i].x, pd.points[i].y))
        .unwrap_or((pd.points[idx].x, pd.points[idx].y));
    let p = &mut pd.points[idx];
    p.mode = match to {
        ConvertTarget::Sharp => CurveMode::Sharp,
        ConvertTarget::Smooth => CurveMode::CatmullRom(0.0),
        ConvertTarget::Arc => {
            // Radius from the chord length; a gentle single-quadrant bulge.
            let dx = p.x - px;
            let dy = p.y - py;
            let chord = (dx * dx + dy * dy).sqrt();
            let r = if chord > 0.0 { chord } else { 10.0 };
            CurveMode::Arc {
                rx: r,
                ry: r,
                sweep: true,
                large: false,
            }
        }
        ConvertTarget::Controls => {
            // Cubic handles at 1/3 and 2/3 of the chord — a straight default the
            // author can then bend. Absolute coordinates (matches `mode=controls`).
            let c1 = (px + (p.x - px) / 3.0, py + (p.y - py) / 3.0);
            let c2 = (px + 2.0 * (p.x - px) / 3.0, py + 2.0 * (p.y - py) / 3.0);
            CurveMode::Controls { c1, c2 }
        }
    };
}

/// Next point in the same contour, wrapping only when the path is closed.
/// `subpath_starts` contains every contour start after the implicit zero.
fn next_point_in_contour(pd: &PathData, idx: usize) -> Option<usize> {
    if idx >= pd.points.len() {
        return None;
    }
    let begin = pd
        .subpath_starts
        .iter()
        .copied()
        .filter(|&start| start <= idx)
        .max()
        .unwrap_or(0);
    let end = pd
        .subpath_starts
        .iter()
        .copied()
        .find(|&start| start > idx)
        .unwrap_or(pd.points.len());
    if idx + 1 < end {
        Some(idx + 1)
    } else if pd.closed && begin < end {
        Some(begin)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(r: f64, h: f64) -> Shape {
        // A rectangle shape; resolve uses coord_space, so geometry is (0,0)..(w,h).
        let _ = (r, h);
        Shape::new("r", Template::Rectangle)
    }

    #[test]
    fn per_corner_rounds_only_listed_corners() {
        let mut s = rect(0.0, 0.0);
        s.operations.push(Operation::RoundCorners {
            radii: CornerRadii::PerCorner(vec![("tl".into(), 10.0), ("br".into(), 0.0)]),
        });
        let pd = s.resolve((100.0, 100.0));
        // tl rounded ⇒ split into tl-in / tl-out (one arc point). tr/bl/br absent
        // or 0 ⇒ left as-is (single sharp point each).
        let names: Vec<&str> = pd.points.iter().map(|p| p.name.as_str()).collect();
        assert!(names.contains(&"tl-out"), "tl rounded: {names:?}");
        assert!(names.contains(&"tr"), "tr stays sharp: {names:?}");
        assert!(names.contains(&"br"), "br=0 stays sharp: {names:?}");
        // The tl-out point carries an Arc mode.
        let tl_out = pd.points.iter().find(|p| p.name == "tl-out").unwrap();
        assert!(matches!(tl_out.mode, CurveMode::Arc { .. }));
    }

    #[test]
    fn uniform_round_corners_still_rounds_all() {
        let mut s = rect(0.0, 0.0);
        s.operations.push(Operation::RoundCorners {
            radii: CornerRadii::uniform(8.0),
        });
        let pd = s.resolve((100.0, 100.0));
        // 4 corners × 2 points each.
        assert_eq!(pd.points.len(), 8);
    }

    #[test]
    fn round_corners_uses_true_radius_on_non_right_angle() {
        let mut s = Shape::new("tri", Template::Path);
        for (name, at) in [
            ("top", (50.0, 0.0)),
            ("br", (100.0, 86.6025403784)),
            ("bl", (0.0, 86.6025403784)),
        ] {
            s.operations.push(Operation::AddPoint {
                name: name.into(),
                at,
                after: None,
                mode: None,
                tension: None,
                arc_rx: None,
                arc_ry: None,
                arc_sweep: None,
                arc_large: None,
                arc_bulge: None,
                control_c1: None,
                control_c2: None,
            });
        }
        s.operations.push(Operation::Close);
        s.operations.push(Operation::RoundCorners {
            radii: CornerRadii::PerCorner(vec![("top".into(), 10.0)]),
        });

        let pd = s.resolve((100.0, 100.0));
        let top_in = pd.points.iter().find(|p| p.name == "top-in").unwrap();
        let tangent_distance = ((top_in.x - 50.0).powi(2) + top_in.y.powi(2)).sqrt();
        assert!((tangent_distance - 17.3205080757).abs() < 1e-6);
        let top_out = pd.points.iter().find(|p| p.name == "top-out").unwrap();
        assert!(matches!(
            top_out.mode,
            CurveMode::Arc { rx, ry, sweep: true, .. }
                if (rx - 10.0).abs() < 1e-6 && (ry - 10.0).abs() < 1e-6
        ));
    }

    #[test]
    fn notch_out_top_pushes_above_edge() {
        // Top edge is at y=0; an OUTWARD tab must produce points with y < 0.
        let mut s = rect(0.0, 0.0);
        s.operations.push(Operation::Notch(NotchSpec {
            edge: NotchEdge::Named(Edge::Top),
            dir: NotchDir::Out,
            shape: NotchShape::Square,
            pos: 0.3,
            width: 20.0,
            depth: 10.0,
        }));
        let pd = s.resolve((100.0, 100.0));
        assert!(
            pd.points.iter().any(|p| p.y < -0.001),
            "outward tab on top edge should have y<0: {:?}",
            pd.points.iter().map(|p| (p.x, p.y)).collect::<Vec<_>>()
        );
    }

    #[test]
    fn notch_in_top_bites_into_shape() {
        let mut s = rect(0.0, 0.0);
        s.operations.push(Operation::Notch(NotchSpec {
            edge: NotchEdge::Named(Edge::Top),
            dir: NotchDir::In,
            shape: NotchShape::Triangle,
            pos: 0.5,
            width: 20.0,
            depth: 10.0,
        }));
        let pd = s.resolve((100.0, 100.0));
        // Inward notch on the top edge ⇒ a point with y > 0 (into the body).
        assert!(pd.points.iter().any(|p| p.y > 0.001));
    }

    #[test]
    fn notch_depth_is_placed_space_under_size() {
        // EXP-5: a `notch depth=D` on a rect placed with a bbox-fit `size=` must
        // measure D on the final canvas. `dir=in` keeps the outer bbox intact, so
        // the fit scale is exact and the mapped depth is exactly D.
        // Authored 200×200 rect, placed size fits scale (sx, sy) = (0.5, 0.25).
        let depth = 6.0;
        let mut s = rect(0.0, 0.0);
        s.operations.push(Operation::Notch(NotchSpec {
            edge: NotchEdge::Named(Edge::Top),
            dir: NotchDir::In,
            shape: NotchShape::Triangle,
            pos: 0.5,
            width: 40.0,
            depth,
        }));
        let scale = (0.5, 0.25);
        let pd = s.resolve_scaled((200.0, 200.0), Some(scale));
        // The tip bites down from the top edge (y=0), so its authored y IS the
        // depth displacement; multiplying by sy gives the placed-space depth.
        let tip_y = pd
            .points
            .iter()
            .find(|p| p.name.contains("tip"))
            .expect("notch tip point")
            .y;
        let placed_depth = tip_y * scale.1;
        assert!(
            (placed_depth - depth).abs() < 1e-6,
            "placed depth should be {depth}, got {placed_depth} (authored tip_y={tip_y})"
        );
        // Contrast: without compensation the authored depth is only `depth`, which
        // would map to `depth × sy` on the canvas — the field-report shrink bug.
        let uncompensated = s.resolve((200.0, 200.0));
        let ut = uncompensated
            .points
            .iter()
            .find(|p| p.name.contains("tip"))
            .expect("notch tip point")
            .y;
        assert!(
            (ut - depth).abs() < 1e-6,
            "authored-space tip is depth, got {ut}"
        );
    }

    #[test]
    fn notch_missing_edge_point_is_noop() {
        // A path with no `tl`/`tr` named points: the named-edge notch is a no-op.
        let mut s = Shape::new("p", Template::Path);
        s.operations.push(Operation::AddPoint {
            name: "x".into(),
            at: (0.0, 0.0),
            after: None,
            mode: None,
            tension: None,
            arc_rx: None,
            arc_ry: None,
            arc_sweep: None,
            arc_large: None,
            arc_bulge: None,
            control_c1: None,
            control_c2: None,
        });
        s.operations.push(Operation::Notch(NotchSpec {
            edge: NotchEdge::Named(Edge::Top),
            dir: NotchDir::Out,
            shape: NotchShape::Square,
            pos: 0.5,
            width: 10.0,
            depth: 5.0,
        }));
        let pd = s.resolve((100.0, 100.0));
        assert_eq!(pd.points.len(), 1); // unchanged, no panic
    }

    #[test]
    fn convert_point_to_all_targets_preserves_position() {
        // Build a 3-point path and convert the middle point to each target.
        let mk = || {
            let mut s = Shape::new("p", Template::Path);
            s.operations.push(Operation::AddPoint {
                name: "a".into(),
                at: (0.0, 0.0),
                after: None,
                mode: None,
                tension: None,
                arc_rx: None,
                arc_ry: None,
                arc_sweep: None,
                arc_large: None,
                arc_bulge: None,
                control_c1: None,
                control_c2: None,
            });
            s.operations.push(Operation::AddPoint {
                name: "b".into(),
                at: (10.0, 20.0),
                after: None,
                mode: None,
                tension: None,
                arc_rx: None,
                arc_ry: None,
                arc_sweep: None,
                arc_large: None,
                arc_bulge: None,
                control_c1: None,
                control_c2: None,
            });
            s
        };
        for to in [
            ConvertTarget::Sharp,
            ConvertTarget::Smooth,
            ConvertTarget::Arc,
            ConvertTarget::Controls,
        ] {
            let mut s = mk();
            s.operations.push(Operation::ConvertPoint {
                point: "b".into(),
                to,
            });
            let pd = s.resolve((100.0, 100.0));
            let b = pd.points.iter().find(|p| p.name == "b").unwrap();
            // Position preserved regardless of target.
            assert_eq!((b.x, b.y), (10.0, 20.0), "{:?} moved the point", to);
            // Mode matches the requested target.
            match (to, &b.mode) {
                (ConvertTarget::Sharp, CurveMode::Sharp) => {}
                (ConvertTarget::Smooth, CurveMode::CatmullRom(_)) => {}
                (ConvertTarget::Arc, CurveMode::Arc { .. }) => {}
                (ConvertTarget::Controls, CurveMode::Controls { .. }) => {}
                other => panic!("convert to {:?} produced {:?}", to, other),
            }
        }
    }

    #[test]
    fn convert_point_missing_is_noop() {
        let mut s = Shape::new("p", Template::Rectangle);
        s.operations.push(Operation::ConvertPoint {
            point: "nope".into(),
            to: ConvertTarget::Arc,
        });
        // Must not panic; geometry unchanged.
        let pd = s.resolve((100.0, 100.0));
        assert_eq!(pd.points.len(), 4);
    }

    #[test]
    fn smooth_corner_curves_both_adjacent_segments() {
        let mut shape = Shape::new("p", Template::Path);
        for (name, at) in [("a", (0.0, 0.0)), ("b", (10.0, 0.0)), ("c", (10.0, 10.0))] {
            shape.operations.push(Operation::AddPoint {
                name: name.into(),
                at,
                after: None,
                mode: None,
                tension: None,
                arc_rx: None,
                arc_ry: None,
                arc_sweep: None,
                arc_large: None,
                arc_bulge: None,
                control_c1: None,
                control_c2: None,
            });
        }
        shape.operations.push(Operation::Close);
        shape.operations.push(Operation::SmoothCorner {
            point: "b".into(),
            tension: Some(0.25),
        });

        let pd = shape.resolve((20.0, 20.0));
        assert!(matches!(pd.points[0].mode, CurveMode::Sharp));
        assert!(matches!(pd.points[1].mode, CurveMode::CatmullRom(t) if (t - 0.25).abs() < 1e-9));
        assert!(matches!(pd.points[2].mode, CurveMode::CatmullRom(t) if (t - 0.25).abs() < 1e-9));
    }

    #[test]
    fn smooth_corner_wraps_inside_closed_subpath() {
        let pd = PathData {
            coord_space: (20.0, 20.0),
            points: vec![
                NamedPoint {
                    name: "a".into(),
                    x: 0.0,
                    y: 0.0,
                    mode: CurveMode::Sharp,
                },
                NamedPoint {
                    name: "b".into(),
                    x: 5.0,
                    y: 0.0,
                    mode: CurveMode::Sharp,
                },
                NamedPoint {
                    name: "c".into(),
                    x: 10.0,
                    y: 10.0,
                    mode: CurveMode::Sharp,
                },
                NamedPoint {
                    name: "d".into(),
                    x: 15.0,
                    y: 10.0,
                    mode: CurveMode::Sharp,
                },
            ],
            closed: true,
            subpath_starts: vec![2],
        };
        assert_eq!(next_point_in_contour(&pd, 1), Some(0));
        assert_eq!(next_point_in_contour(&pd, 3), Some(2));
    }

    #[test]
    fn rectangle_template_has_four_corners() {
        let shape = Shape::new("bg", Template::Rectangle);
        let pd = shape.resolve((100.0, 100.0));
        assert_eq!(pd.points.len(), 4);
        assert!(pd.closed);
        assert_eq!(pd.points[0].name, "tl");
        assert_eq!(pd.points[1].name, "tr");
    }

    #[test]
    fn ellipse_template_has_smooth_cardinals() {
        let shape = Shape::new("circle", Template::Ellipse);
        let pd = shape.resolve((100.0, 100.0));
        assert_eq!(pd.points.len(), 4);
        assert!(pd.closed);
        // Tension should be 1 - 4*(√2-1) ≈ -0.6569 for a proper circle approximation.
        let expected = 1.0 - 4.0 * (std::f64::consts::SQRT_2 - 1.0);
        assert!(
            matches!(pd.points[0].mode, CurveMode::CatmullRom(t) if (t - expected).abs() < 0.001)
        );
    }

    #[test]
    fn path_template_starts_empty() {
        let shape = Shape::new("custom", Template::Path);
        let pd = shape.resolve((100.0, 100.0));
        assert!(pd.points.is_empty());
        assert!(!pd.closed);
    }

    #[test]
    fn addpoint_builds_path() {
        let mut shape = Shape::new("stem", Template::Path);
        shape.operations.push(Operation::AddPoint {
            name: "base".to_string(),
            at: (200.0, 385.0),
            after: None,
            mode: None,
            tension: None,
            arc_rx: None,
            arc_ry: None,
            arc_sweep: None,
            arc_large: None,
            arc_bulge: None,
            control_c1: None,
            control_c2: None,
        });
        shape.operations.push(Operation::AddPoint {
            name: "mid".to_string(),
            at: (192.0, 300.0),
            after: Some("base".to_string()),
            mode: Some(PointMode::CatmullRom),
            tension: Some(0.3),
            arc_rx: None,
            arc_ry: None,
            arc_sweep: None,
            arc_large: None,
            arc_bulge: None,
            control_c1: None,
            control_c2: None,
        });
        let pd = shape.resolve((400.0, 400.0));
        assert_eq!(pd.points.len(), 2);
        assert_eq!(pd.points[0].name, "base");
        assert_eq!(pd.points[1].name, "mid");
        assert!(matches!(pd.points[1].mode, CurveMode::CatmullRom(t) if (t - 0.3).abs() < 0.01));
    }

    #[test]
    fn addpoint_controls_builds_bezier_curve() {
        let mut shape = Shape::new("curve", Template::Path);
        shape.operations.push(Operation::AddPoint {
            name: "start".to_string(),
            at: (0.0, 0.0),
            after: None,
            mode: None,
            tension: None,
            arc_rx: None,
            arc_ry: None,
            arc_sweep: None,
            arc_large: None,
            arc_bulge: None,
            control_c1: None,
            control_c2: None,
        });
        shape.operations.push(Operation::AddPoint {
            name: "end".to_string(),
            at: (100.0, 100.0),
            after: Some("start".to_string()),
            mode: Some(PointMode::Controls),
            tension: None,
            arc_rx: None,
            arc_ry: None,
            arc_sweep: None,
            arc_large: None,
            arc_bulge: None,
            control_c1: Some((25.0, 0.0)),
            control_c2: Some((100.0, 75.0)),
        });
        let pd = shape.resolve((200.0, 200.0));
        assert_eq!(pd.points.len(), 2);
        assert!(matches!(
            pd.points[1].mode,
            CurveMode::Controls { c1, c2 }
                if c1 == (25.0, 0.0) && c2 == (100.0, 75.0)
        ));
    }

    #[test]
    fn movepoint_to_repositions() {
        let mut shape = Shape::new("line", Template::Line);
        shape.operations.push(Operation::MovePointTo {
            point: "start".to_string(),
            to: (5.0, 10.0),
        });
        let pd = shape.resolve((100.0, 100.0));
        assert_eq!(pd.points[0].x, 5.0);
        assert_eq!(pd.points[0].y, 10.0);
    }

    #[test]
    fn pullpoint_dir_moves_relative() {
        let mut shape = Shape::new("e", Template::Ellipse);
        shape.operations.push(Operation::PullPointDir {
            point: "top".to_string(),
            dir: Direction::Up,
            amount: RelativeSize(15.0),
        });
        let pd = shape.resolve((100.0, 100.0));
        // top was at (50, 0), moving up 15% of 100 = -15 → y = -15
        assert!((pd.points[0].y - (-15.0)).abs() < 0.01);
    }

    #[test]
    fn close_open_toggle() {
        let mut shape = Shape::new("p", Template::Path);
        shape.operations.push(Operation::AddPoint {
            name: "a".to_string(),
            at: (0.0, 0.0),
            after: None,
            mode: None,
            tension: None,
            arc_rx: None,
            arc_ry: None,
            arc_sweep: None,
            arc_large: None,
            arc_bulge: None,
            control_c1: None,
            control_c2: None,
        });
        shape.operations.push(Operation::AddPoint {
            name: "b".to_string(),
            at: (100.0, 0.0),
            after: None,
            mode: None,
            tension: None,
            arc_rx: None,
            arc_ry: None,
            arc_sweep: None,
            arc_large: None,
            arc_bulge: None,
            control_c1: None,
            control_c2: None,
        });
        shape.operations.push(Operation::AddPoint {
            name: "c".to_string(),
            at: (50.0, 100.0),
            after: None,
            mode: None,
            tension: None,
            arc_rx: None,
            arc_ry: None,
            arc_sweep: None,
            arc_large: None,
            arc_bulge: None,
            control_c1: None,
            control_c2: None,
        });
        shape.operations.push(Operation::Close);
        let pd = shape.resolve((100.0, 100.0));
        assert!(pd.closed);

        shape.operations.push(Operation::Open);
        let pd = shape.resolve((100.0, 100.0));
        assert!(!pd.closed);
    }

    #[test]
    fn fill_stroke_attrs() {
        let mut shape = Shape::new("bg", Template::Rectangle);
        shape
            .operations
            .push(Operation::Fill(Color::Hex("#faf6f0".to_string())));
        shape
            .operations
            .push(Operation::Stroke(Color::Hex("#000000".to_string())));
        shape
            .operations
            .push(Operation::StrokeWidth(AbsoluteSize(2.0)));

        assert_eq!(shape.fill(), Some(&Color::Hex("#faf6f0".to_string())));
        assert_eq!(shape.stroke(), Some(&Color::Hex("#000000".to_string())));
        assert_eq!(shape.stroke_width(), Some(2.0));
    }
}
