/// v3 type system — parsed from DSL tokens, validated at parse time.
use crate::error::{Result, StrokError};
use std::fmt;

// ── Scalar types ──────────────────────────────────────────────────────

/// Pixels. Bare number in DSL: `400`, `2.5`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AbsoluteSize(pub f64);

/// Relative to context. Number + `%` in DSL: `40%`, `120%`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct RelativeSize(pub f64);

/// Degrees. Number + `deg` in DSL: `45deg`, `-30deg`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Rotation(pub f64);

/// Unitless fraction 0–1. Bare number in DSL: `0.3`, `1`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct NormalizedAmount(pub f64);

/// Width × Height in pixels. `WxH` in DSL: `400x400`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Dimension {
    pub w: f64,
    pub h: f64,
}

/// CSS hex color, gradient, or a palette token reference (`$name`).
#[derive(Debug, Clone, PartialEq)]
pub enum Color {
    Hex(String),
    None,
    /// The CSS `currentColor` keyword. Emitted verbatim into SVG so the icon
    /// inherits the surrounding `color`; substituted with a concrete color at
    /// raster time (`render --color`, default black).
    CurrentColor,
    /// Reference to a palette token, written `$name`. Resolved to a concrete
    /// color at render time against the active colorscheme (see `resolve::apply_scheme`).
    Token(String),
    LinearGradient(LinearGradient),
    RadialGradient(RadialGradient),
}

/// A color stop in a gradient.
#[derive(Debug, Clone, PartialEq)]
pub struct GradientStop {
    /// "#rrggbb", "#rrggbbaa", or "transparent"
    pub color: String,
    /// 0.0–1.0, None = auto-distribute
    pub position: Option<f64>,
}

/// Linear gradient: from edge to edge with color stops.
#[derive(Debug, Clone, PartialEq)]
pub struct LinearGradient {
    pub from: GradientEdge,
    pub to: GradientEdge,
    pub stops: Vec<GradientStop>,
}

/// Radial gradient: center + radius + color stops.
#[derive(Debug, Clone, PartialEq)]
pub struct RadialGradient {
    pub center: GradientEdge,
    /// Percentage (e.g. 80.0 for 80%)
    pub radius: f64,
    pub stops: Vec<GradientStop>,
}

/// 9-point grid for gradient positioning.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GradientEdge {
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

/// Cardinal direction keyword.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    Left,
    Right,
    Up,
    Down,
}

/// Flip axis for placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flip {
    X,
    Y,
    XY,
}

/// Qualified point reference: `shape.point`.
#[derive(Debug, Clone, PartialEq)]
pub struct PointRef {
    pub shape: String,
    pub point: String,
}

/// Two-point segment reference: `shape.{p1,p2}`.
#[derive(Debug, Clone, PartialEq)]
pub struct SegmentRef {
    pub shape: String,
    pub p1: String,
    pub p2: String,
}

/// Hierarchical path selector: `rose.bloom.petal-1`.
#[derive(Debug, Clone, PartialEq)]
pub struct Selector(pub Vec<String>);

/// Placement side for parametric placement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    Left,
    Right,
}

/// Stroke line cap style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineCap {
    Round,
    Butt,
    Square,
}

/// Stroke line join style.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineJoin {
    Miter,
    Round,
    Bevel,
}

/// SVG fill-rule: how the interior of a self-intersecting / holey path is
/// determined. Prerequisite for correct holes and for P2 boolean operations.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FillRule {
    /// `nonzero` (SVG default): winding-number rule.
    NonZero,
    /// `even-odd`: a point is inside iff a ray crosses an odd number of edges.
    EvenOdd,
}

/// SVG text-anchor alignment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TextAnchor {
    Start,
    Middle,
    End,
}

/// Layout mode for repeat.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatLayout {
    Ring,
    Line,
    Arc,
}

/// Sculpt axis constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SculptAxis {
    Xy,
    Tangent,
    Normal,
}

/// Reconnect mode for point deletion.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReconnectMode {
    Auto,
    Line,
    Smooth,
}

/// Point insertion mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PointMode {
    Sharp,
    CatmullRom,
    Arc,
    Controls,
    ControlsRelative,
}

/// Which side an `mode=arc` segment bulges toward, relative to the direction of
/// travel (previous point → this point). Unlike the raw SVG `sweep` flag, this is
/// **independent of point order**: reversing the points keeps the bulge on the
/// same visual side. `Left`/`Right` are taken in screen space (y-down).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArcBulge {
    Left,
    Right,
}

impl fmt::Display for ArcBulge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ArcBulge::Left => write!(f, "left"),
            ArcBulge::Right => write!(f, "right"),
        }
    }
}

impl ArcBulge {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "left" => Ok(ArcBulge::Left),
            "right" => Ok(ArcBulge::Right),
            _ => Err(StrokError::ParseError(format!(
                "bulge must be left or right, got '{}'",
                s
            ))),
        }
    }

    /// Resolve to an SVG sweep flag given the travel vector (dx, dy) from the
    /// previous point to this point, in screen space (y-down).
    ///
    /// In SVG's y-down coordinate space a `sweep=1` arc curves to the right of the
    /// direction of travel, `sweep=0` to the left. We derive the flag from the
    /// travel direction so the chosen visual side is preserved regardless of the
    /// order the points were authored in. For a degenerate (zero-length) travel
    /// vector we fall back to the historical default (sweep=1) so behavior stays
    /// well-defined.
    pub fn to_sweep_flag(self, dx: f64, dy: f64) -> bool {
        if dx == 0.0 && dy == 0.0 {
            return true;
        }
        match self {
            ArcBulge::Right => true,
            ArcBulge::Left => false,
        }
    }
}

// ── Display impls ─────────────────────────────────────────────────────

impl fmt::Display for AbsoluteSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", fmt_num(self.0))
    }
}

impl fmt::Display for RelativeSize {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}%", fmt_num(self.0))
    }
}

impl fmt::Display for Rotation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}deg", fmt_num(self.0))
    }
}

impl fmt::Display for NormalizedAmount {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", fmt_num(self.0))
    }
}

impl fmt::Display for Dimension {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}x{}", fmt_num(self.w), fmt_num(self.h))
    }
}

impl fmt::Display for Color {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Color::Hex(s) => write!(f, "{}", s),
            Color::None => write!(f, "none"),
            Color::CurrentColor => write!(f, "currentColor"),
            Color::Token(t) => write!(f, "${}", t),
            Color::LinearGradient(g) => write!(f, "{}", g),
            Color::RadialGradient(g) => write!(f, "{}", g),
        }
    }
}

impl fmt::Display for GradientEdge {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            GradientEdge::TopLeft => write!(f, "tl"),
            GradientEdge::Top => write!(f, "top"),
            GradientEdge::TopRight => write!(f, "tr"),
            GradientEdge::Left => write!(f, "left"),
            GradientEdge::Center => write!(f, "center"),
            GradientEdge::Right => write!(f, "right"),
            GradientEdge::BottomLeft => write!(f, "bl"),
            GradientEdge::Bottom => write!(f, "bottom"),
            GradientEdge::BottomRight => write!(f, "br"),
        }
    }
}

impl fmt::Display for GradientStop {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.color)?;
        if let Some(pos) = self.position {
            write!(f, " {}%", fmt_num(pos * 100.0))?;
        }
        Ok(())
    }
}

impl fmt::Display for LinearGradient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "linear({}, {}", self.from, self.to)?;
        for stop in &self.stops {
            write!(f, ", {}", stop)?;
        }
        write!(f, ")")
    }
}

impl fmt::Display for RadialGradient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "radial({}, {}%", self.center, fmt_num(self.radius))?;
        for stop in &self.stops {
            write!(f, ", {}", stop)?;
        }
        write!(f, ")")
    }
}

impl GradientEdge {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "tl" => Ok(GradientEdge::TopLeft),
            "top" => Ok(GradientEdge::Top),
            "tr" => Ok(GradientEdge::TopRight),
            "left" => Ok(GradientEdge::Left),
            "center" => Ok(GradientEdge::Center),
            "right" => Ok(GradientEdge::Right),
            "bl" => Ok(GradientEdge::BottomLeft),
            "bottom" => Ok(GradientEdge::Bottom),
            "br" => Ok(GradientEdge::BottomRight),
            _ => Err(StrokError::ParseError(format!(
                "expected gradient edge (tl/top/tr/left/center/right/bl/bottom/br), got '{}'",
                s
            ))),
        }
    }

    /// Returns (x%, y%) for SVG gradient coordinates.
    pub fn to_svg_percent(&self) -> (f64, f64) {
        match self {
            GradientEdge::TopLeft => (0.0, 0.0),
            GradientEdge::Top => (50.0, 0.0),
            GradientEdge::TopRight => (100.0, 0.0),
            GradientEdge::Left => (0.0, 50.0),
            GradientEdge::Center => (50.0, 50.0),
            GradientEdge::Right => (100.0, 50.0),
            GradientEdge::BottomLeft => (0.0, 100.0),
            GradientEdge::Bottom => (50.0, 100.0),
            GradientEdge::BottomRight => (100.0, 100.0),
        }
    }
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Direction::Left => write!(f, "left"),
            Direction::Right => write!(f, "right"),
            Direction::Up => write!(f, "up"),
            Direction::Down => write!(f, "down"),
        }
    }
}

impl fmt::Display for Flip {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Flip::X => write!(f, "x"),
            Flip::Y => write!(f, "y"),
            Flip::XY => write!(f, "xy"),
        }
    }
}

impl fmt::Display for PointRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.shape, self.point)
    }
}

impl fmt::Display for SegmentRef {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{{{},{}}}", self.shape, self.p1, self.p2)
    }
}

impl fmt::Display for Selector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0.join("."))
    }
}

impl fmt::Display for Side {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Side::Left => write!(f, "left"),
            Side::Right => write!(f, "right"),
        }
    }
}

impl fmt::Display for LineCap {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineCap::Round => write!(f, "round"),
            LineCap::Butt => write!(f, "butt"),
            LineCap::Square => write!(f, "square"),
        }
    }
}

impl fmt::Display for LineJoin {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LineJoin::Miter => write!(f, "miter"),
            LineJoin::Round => write!(f, "round"),
            LineJoin::Bevel => write!(f, "bevel"),
        }
    }
}

impl fmt::Display for FillRule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FillRule::NonZero => write!(f, "nonzero"),
            FillRule::EvenOdd => write!(f, "even-odd"),
        }
    }
}

impl FillRule {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "nonzero" => Ok(FillRule::NonZero),
            // Accept both the DSL spelling and the SVG attribute spelling.
            "even-odd" | "evenodd" => Ok(FillRule::EvenOdd),
            _ => Err(StrokError::ParseError(format!(
                "fill-rule must be nonzero or even-odd, got '{}'",
                s
            ))),
        }
    }

    /// The SVG attribute value (`evenodd`, not `even-odd`).
    pub fn svg_value(&self) -> &'static str {
        match self {
            FillRule::NonZero => "nonzero",
            FillRule::EvenOdd => "evenodd",
        }
    }
}

impl fmt::Display for TextAnchor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TextAnchor::Start => write!(f, "start"),
            TextAnchor::Middle => write!(f, "middle"),
            TextAnchor::End => write!(f, "end"),
        }
    }
}

impl fmt::Display for RepeatLayout {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RepeatLayout::Ring => write!(f, "ring"),
            RepeatLayout::Line => write!(f, "line"),
            RepeatLayout::Arc => write!(f, "arc"),
        }
    }
}

impl fmt::Display for SculptAxis {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SculptAxis::Xy => write!(f, "xy"),
            SculptAxis::Tangent => write!(f, "tangent"),
            SculptAxis::Normal => write!(f, "normal"),
        }
    }
}

impl fmt::Display for ReconnectMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ReconnectMode::Auto => write!(f, "auto"),
            ReconnectMode::Line => write!(f, "line"),
            ReconnectMode::Smooth => write!(f, "smooth"),
        }
    }
}

impl fmt::Display for PointMode {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            PointMode::Sharp => write!(f, "sharp"),
            PointMode::CatmullRom => write!(f, "catmull-rom"),
            PointMode::Arc => write!(f, "arc"),
            PointMode::Controls => write!(f, "controls"),
            PointMode::ControlsRelative => write!(f, "controls-relative"),
        }
    }
}

// ── Parse impls ───────────────────────────────────────────────────────

impl AbsoluteSize {
    pub fn parse(s: &str) -> Result<Self> {
        if s.ends_with('%') || s.ends_with("deg") {
            return Err(StrokError::ParseError(format!(
                "expected AbsoluteSize (bare number), got '{}'",
                s
            )));
        }
        let v: f64 = s
            .parse()
            .map_err(|_| StrokError::ParseError(format!("invalid number: '{}'", s)))?;
        Ok(AbsoluteSize(v))
    }
}

impl RelativeSize {
    pub fn parse(s: &str) -> Result<Self> {
        if !s.ends_with('%') {
            return Err(StrokError::ParseError(format!(
                "expected RelativeSize (number%), got '{}'",
                s
            )));
        }
        let v: f64 = s[..s.len() - 1]
            .parse()
            .map_err(|_| StrokError::ParseError(format!("invalid percentage: '{}'", s)))?;
        Ok(RelativeSize(v))
    }
}

impl Rotation {
    pub fn parse(s: &str) -> Result<Self> {
        // Degrees are the only unit, so the `deg` suffix is optional:
        // `rotation=45` and `rotation=45deg` are equivalent. This matches the
        // CLI, which already accepts a bare number.
        let num = s.strip_suffix("deg").unwrap_or(s);
        let v: f64 = num.parse().map_err(|_| {
            StrokError::ParseError(format!(
                "expected Rotation (a number of degrees, e.g. 45 or 45deg), got '{}'",
                s
            ))
        })?;
        Ok(Rotation(v))
    }
}

impl NormalizedAmount {
    pub fn parse(s: &str) -> Result<Self> {
        if s.ends_with('%') || s.ends_with("deg") {
            return Err(StrokError::ParseError(format!(
                "expected NormalizedAmount (0-1), got '{}'",
                s
            )));
        }
        let v: f64 = s
            .parse()
            .map_err(|_| StrokError::ParseError(format!("invalid number: '{}'", s)))?;
        if !(0.0..=1.0).contains(&v) {
            return Err(StrokError::ParseError(format!(
                "NormalizedAmount must be 0-1, got {}",
                v
            )));
        }
        Ok(NormalizedAmount(v))
    }
}

impl Dimension {
    pub fn parse(s: &str) -> Result<Self> {
        let parts: Vec<&str> = s.split('x').collect();
        if parts.len() != 2 {
            return Err(StrokError::ParseError(format!(
                "expected Dimension (WxH), got '{}'",
                s
            )));
        }
        let w: f64 = parts[0]
            .parse()
            .map_err(|_| StrokError::ParseError(format!("invalid width in dimension: '{}'", s)))?;
        let h: f64 = parts[1]
            .parse()
            .map_err(|_| StrokError::ParseError(format!("invalid height in dimension: '{}'", s)))?;
        Ok(Dimension { w, h })
    }
}

impl Color {
    pub fn parse(s: &str) -> Result<Self> {
        if s == "none" {
            return Ok(Color::None);
        }
        // CSS `currentColor` — case-insensitive, as in CSS. Lets icons inherit the
        // page's `color`; kept verbatim in SVG and substituted only at raster time.
        if s.eq_ignore_ascii_case("currentcolor") {
            return Ok(Color::CurrentColor);
        }
        if s == "transparent" {
            return Ok(Color::Hex("#00000000".to_string()));
        }
        if let Some(token) = s.strip_prefix('$') {
            // A single `.` is allowed for the dotted category spelling
            // (`$color.accent` — same token as `$accent`).
            if token.is_empty()
                || token.starts_with('.')
                || token.ends_with('.')
                || token.matches('.').count() > 1
                || !token
                    .chars()
                    .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
            {
                return Err(StrokError::ParseError(format!(
                    "'{}' is not a valid color token — use $name or $category.name (letters, digits, -, _)",
                    s
                )));
            }
            return Ok(Color::Token(token.to_string()));
        }
        if !s.starts_with('#') {
            return Err(StrokError::ParseError(format!(
                "'{}' is not a valid color — use #rrggbb (e.g. #c8863a) or none",
                s
            )));
        }
        let hex = &s[1..];
        if hex.len() != 6 && hex.len() != 8 {
            return Err(StrokError::ParseError(format!(
                "'{}' is not a valid color — hex must be 6 digits (#rrggbb) or 8 digits (#rrggbbaa)",
                s
            )));
        }
        if !hex.chars().all(|c| c.is_ascii_hexdigit()) {
            return Err(StrokError::ParseError(format!(
                "invalid hex digits in color: '{}'",
                s
            )));
        }
        Ok(Color::Hex(s.to_string()))
    }
}

impl Direction {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "left" => Ok(Direction::Left),
            "right" => Ok(Direction::Right),
            "up" => Ok(Direction::Up),
            "down" => Ok(Direction::Down),
            _ => Err(StrokError::ParseError(format!(
                "expected direction (left/right/up/down), got '{}'",
                s
            ))),
        }
    }
}

impl Flip {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "x" => Ok(Flip::X),
            "y" => Ok(Flip::Y),
            "xy" => Ok(Flip::XY),
            _ => Err(StrokError::ParseError(format!(
                "expected flip (x/y/xy), got '{}'",
                s
            ))),
        }
    }
}

impl PointRef {
    pub fn parse(s: &str) -> Result<Self> {
        let dot = s.find('.').ok_or_else(|| {
            StrokError::ParseError(format!("expected PointRef (shape.point), got '{}'", s))
        })?;
        let shape = &s[..dot];
        let point = &s[dot + 1..];
        if shape.is_empty() || point.is_empty() {
            return Err(StrokError::ParseError(format!("invalid PointRef: '{}'", s)));
        }
        Ok(PointRef {
            shape: shape.to_string(),
            point: point.to_string(),
        })
    }
}

impl SegmentRef {
    pub fn parse(s: &str) -> Result<Self> {
        // shape.{p1,p2}
        let dot = s.find('.').ok_or_else(|| {
            StrokError::ParseError(format!(
                "expected SegmentRef (shape.{{p1,p2}}), got '{}'",
                s
            ))
        })?;
        let shape = &s[..dot];
        let rest = &s[dot + 1..];
        if !rest.starts_with('{') || !rest.ends_with('}') {
            return Err(StrokError::ParseError(format!(
                "expected SegmentRef (shape.{{p1,p2}}), got '{}'",
                s
            )));
        }
        let inner = &rest[1..rest.len() - 1];
        let parts: Vec<&str> = inner.split(',').collect();
        if parts.len() != 2 {
            return Err(StrokError::ParseError(format!(
                "expected two points in SegmentRef, got '{}'",
                s
            )));
        }
        Ok(SegmentRef {
            shape: shape.to_string(),
            p1: parts[0].trim().to_string(),
            p2: parts[1].trim().to_string(),
        })
    }
}

impl Selector {
    pub fn parse(s: &str) -> Result<Self> {
        let parts: Vec<String> = s.split('.').map(|p| p.to_string()).collect();
        if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
            return Err(StrokError::ParseError(format!("invalid selector: '{}'", s)));
        }
        Ok(Selector(parts))
    }
}

impl Side {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "left" => Ok(Side::Left),
            "right" => Ok(Side::Right),
            _ => Err(StrokError::ParseError(format!(
                "expected side (left/right), got '{}'",
                s
            ))),
        }
    }
}

impl LineCap {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "round" => Ok(LineCap::Round),
            "butt" => Ok(LineCap::Butt),
            "square" => Ok(LineCap::Square),
            _ => Err(StrokError::ParseError(format!(
                "expected linecap (round/butt/square), got '{}'",
                s
            ))),
        }
    }
}

impl LineJoin {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "miter" => Ok(LineJoin::Miter),
            "round" => Ok(LineJoin::Round),
            "bevel" => Ok(LineJoin::Bevel),
            _ => Err(StrokError::ParseError(format!(
                "expected linejoin (miter/round/bevel), got '{}'",
                s
            ))),
        }
    }
}

impl TextAnchor {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "start" => Ok(TextAnchor::Start),
            "middle" => Ok(TextAnchor::Middle),
            "end" => Ok(TextAnchor::End),
            _ => Err(StrokError::ParseError(format!(
                "expected text-anchor (start/middle/end), got '{}'",
                s
            ))),
        }
    }
}

impl RepeatLayout {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "ring" => Ok(RepeatLayout::Ring),
            "line" => Ok(RepeatLayout::Line),
            "arc" => Ok(RepeatLayout::Arc),
            _ => Err(StrokError::ParseError(format!(
                "expected layout (ring/line/arc), got '{}'",
                s
            ))),
        }
    }
}

impl SculptAxis {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "xy" => Ok(SculptAxis::Xy),
            "tangent" => Ok(SculptAxis::Tangent),
            "normal" => Ok(SculptAxis::Normal),
            _ => Err(StrokError::ParseError(format!(
                "expected axis (xy/tangent/normal), got '{}'",
                s
            ))),
        }
    }
}

impl ReconnectMode {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "auto" => Ok(ReconnectMode::Auto),
            "line" => Ok(ReconnectMode::Line),
            "smooth" => Ok(ReconnectMode::Smooth),
            _ => Err(StrokError::ParseError(format!(
                "expected reconnect (auto/line/smooth), got '{}'",
                s
            ))),
        }
    }
}

impl PointMode {
    pub fn parse(s: &str) -> Result<Self> {
        match s {
            "sharp" => Ok(PointMode::Sharp),
            "catmull-rom" => Ok(PointMode::CatmullRom),
            "arc" => Ok(PointMode::Arc),
            "controls" => Ok(PointMode::Controls),
            "controls-relative" => Ok(PointMode::ControlsRelative),
            _ => Err(StrokError::ParseError(format!(
                "expected mode (sharp/catmull-rom/arc/controls/controls-relative), got '{}'",
                s
            ))),
        }
    }
}

impl Color {
    /// Parse a gradient expression: `radial(...)` or `linear(...)`.
    /// The input should be the full expression including the outer parens.
    pub fn parse_gradient(s: &str) -> Result<Self> {
        let s = s.trim();
        if let Some(inner) = s.strip_prefix("radial(").and_then(|s| s.strip_suffix(')')) {
            let parts = split_gradient_args(inner);
            if parts.len() < 4 {
                return Err(StrokError::ParseError(
                    "radial() requires center, radius, and at least 2 color stops".to_string(),
                ));
            }
            let center = GradientEdge::parse(parts[0].trim())?;
            let radius_str = parts[1].trim();
            if !radius_str.ends_with('%') {
                return Err(StrokError::ParseError(format!(
                    "radial radius must be a percentage, got '{}'",
                    radius_str
                )));
            }
            let radius: f64 = radius_str[..radius_str.len() - 1]
                .parse()
                .map_err(|_| StrokError::ParseError(format!("invalid radius: '{}'", radius_str)))?;
            let stops = parse_gradient_stops(&parts[2..])?;
            Ok(Color::RadialGradient(RadialGradient {
                center,
                radius,
                stops,
            }))
        } else if let Some(inner) = s.strip_prefix("linear(").and_then(|s| s.strip_suffix(')')) {
            let parts = split_gradient_args(inner);
            if parts.len() < 4 {
                return Err(StrokError::ParseError(
                    "linear() requires from, to, and at least 2 color stops".to_string(),
                ));
            }
            let from = GradientEdge::parse(parts[0].trim())?;
            let to = GradientEdge::parse(parts[1].trim())?;
            let stops = parse_gradient_stops(&parts[2..])?;
            Ok(Color::LinearGradient(LinearGradient { from, to, stops }))
        } else {
            Err(StrokError::ParseError(format!(
                "expected radial(...) or linear(...), got '{}'",
                s
            )))
        }
    }
}

/// Split gradient arguments by comma, respecting that color+position are one arg.
fn split_gradient_args(s: &str) -> Vec<&str> {
    s.split(',').collect()
}

/// Parse gradient stop strings like "#ff0000", "#ff0000 50%", "transparent".
fn parse_gradient_stops(parts: &[&str]) -> Result<Vec<GradientStop>> {
    let mut stops = Vec::new();
    for part in parts {
        let part = part.trim();
        let tokens: Vec<&str> = part.split_whitespace().collect();
        if tokens.is_empty() {
            return Err(StrokError::ParseError("empty gradient stop".to_string()));
        }
        let color_str = tokens[0];
        // Resolve "transparent" to last-color-at-zero-opacity or black transparent
        let color = if color_str == "transparent" {
            "transparent".to_string()
        } else if let Some(hex) = color_str.strip_prefix('#') {
            if (hex.len() != 6 && hex.len() != 8) || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
                return Err(StrokError::ParseError(format!(
                    "invalid color in gradient stop: '{}'",
                    color_str
                )));
            }
            color_str.to_string()
        } else {
            return Err(StrokError::ParseError(format!(
                "invalid gradient stop color: '{}' — use #rrggbb or transparent",
                color_str
            )));
        };
        let position = if tokens.len() > 1 {
            let pos_str = tokens[1];
            if !pos_str.ends_with('%') {
                return Err(StrokError::ParseError(format!(
                    "gradient stop position must be a percentage, got '{}'",
                    pos_str
                )));
            }
            let pct: f64 = pos_str[..pos_str.len() - 1]
                .parse()
                .map_err(|_| StrokError::ParseError(format!("invalid position: '{}'", pos_str)))?;
            Some(pct / 100.0)
        } else {
            None
        };
        stops.push(GradientStop { color, position });
    }
    Ok(stops)
}

// ── Helpers ───────────────────────────────────────────────────────────

/// Parse a `x,y` coordinate pair.
pub fn parse_point_coord(s: &str) -> Result<(f64, f64)> {
    let parts: Vec<&str> = s.split(',').collect();
    if parts.len() != 2 {
        return Err(StrokError::ParseError(format!(
            "expected PointCoord (x,y), got '{}'",
            s
        )));
    }
    let x: f64 = parts[0]
        .trim()
        .parse()
        .map_err(|_| StrokError::ParseError(format!("invalid x in coordinate: '{}'", s)))?;
    let y: f64 = parts[1]
        .trim()
        .parse()
        .map_err(|_| StrokError::ParseError(format!("invalid y in coordinate: '{}'", s)))?;
    Ok((x, y))
}

/// Parse a value that can be either AbsoluteSize or RelativeSize.
pub fn parse_size(s: &str) -> Result<SizeValue> {
    if s.ends_with('%') {
        Ok(SizeValue::Relative(RelativeSize::parse(s)?))
    } else {
        Ok(SizeValue::Absolute(AbsoluteSize::parse(s)?))
    }
}

/// A value that can be absolute or relative.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum SizeValue {
    Absolute(AbsoluteSize),
    Relative(RelativeSize),
}

impl fmt::Display for SizeValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SizeValue::Absolute(a) => write!(f, "{}", a),
            SizeValue::Relative(r) => write!(f, "{}", r),
        }
    }
}

/// Parse a signed absolute size (for dx/dy values).
pub fn parse_signed(s: &str) -> Result<f64> {
    s.parse::<f64>()
        .map_err(|_| StrokError::ParseError(format!("invalid number: '{}'", s)))
}

/// Validate an identifier: `[A-Za-z][A-Za-z0-9_-]*`.
///
/// Kebab-case remains the recommended authoring convention, but rejecting
/// camelCase and snake_case made the parser needlessly brittle for agents (and
/// invalidated otherwise-correct SVG-compatible names found in the corpus).
pub fn validate_ident(s: &str) -> Result<()> {
    if s.is_empty() {
        return Err(StrokError::ParseError("empty identifier".to_string()));
    }
    let mut chars = s.chars();
    // `s` is non-empty (checked above), so `next()` always yields.
    let Some(first) = chars.next() else {
        return Err(StrokError::ParseError("empty identifier".to_string()));
    };
    if !first.is_ascii_alphabetic() {
        return Err(StrokError::ParseError(format!(
            "identifier must start with a letter: '{}'",
            s
        )));
    }
    for c in chars {
        if !c.is_ascii_alphanumeric() && c != '-' && c != '_' {
            return Err(StrokError::ParseError(format!(
                "identifier contains invalid character '{}': '{}'",
                c, s
            )));
        }
    }
    Ok(())
}

/// Format a number for emit: ~6 significant figures, trailing zeros trimmed,
/// never scientific notation, "-0" normalized to "0".
///
/// Fixes Cause C (the old `{:.4}` truncation quantized to 1e-4 of a *document
/// unit*, becoming visible sub-pixel drift after an 8–64× render scale). The
/// decimal-place budget is chosen *relative to the magnitude* of the value so
/// precision scales with the coordinate: large coords keep ~6 significant
/// figures, small coords keep enough fraction digits to be sub-pixel exact at
/// any sane render scale. Output is canonical (a re-format is idempotent) so the
/// round-trip invariant holds.
pub fn fmt_num(n: f64) -> String {
    // Non-finite values would otherwise emit "inf"/"NaN" into a `d` string;
    // collapse them to 0 so emitted SVG is always well-formed.
    if !n.is_finite() || n == 0.0 {
        return "0".to_string();
    }
    let abs = n.abs();
    // Number of integer digits (>=1). For abs < 1 this is 1 conceptually; we
    // grant extra fraction digits below.
    let int_digits = if abs >= 1.0 {
        abs.log10().floor() as i32 + 1
    } else {
        0
    };
    // Target 6 significant figures. Fraction digits = 6 - integer digits,
    // clamped to a sane window. For sub-1 values we add the count of leading
    // zeros after the decimal so the 6 sig-figs land on real digits.
    let mut frac_digits = 6 - int_digits;
    if abs < 1.0 {
        // leading zeros after the point, e.g. 0.004 → 2 leading zeros.
        let lead = (-abs.log10().floor()) as i32 - 1;
        frac_digits = 6 + lead.max(0);
    }
    let frac_digits = frac_digits.clamp(0, 10) as usize;
    let s = format!("{:.*}", frac_digits, n);
    let s = if s.contains('.') {
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    } else {
        s
    };
    // Guard against a "-0" surviving the rounding (e.g. -0.00001 → "-0").
    if s == "-0" {
        "0".to_string()
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_absolute_size() {
        assert_eq!(AbsoluteSize::parse("400").unwrap().0, 400.0);
        assert_eq!(AbsoluteSize::parse("2.5").unwrap().0, 2.5);
        assert!(AbsoluteSize::parse("40%").is_err());
        assert!(AbsoluteSize::parse("45deg").is_err());
    }

    #[test]
    fn parse_relative_size() {
        assert_eq!(RelativeSize::parse("40%").unwrap().0, 40.0);
        assert!(RelativeSize::parse("40").is_err());
    }

    #[test]
    fn parse_rotation() {
        assert_eq!(Rotation::parse("45deg").unwrap().0, 45.0);
        assert_eq!(Rotation::parse("-30deg").unwrap().0, -30.0);
        // The `deg` suffix is optional — a bare number is degrees.
        assert_eq!(Rotation::parse("45").unwrap().0, 45.0);
        assert_eq!(Rotation::parse("-30").unwrap().0, -30.0);
        assert!(Rotation::parse("abc").is_err());
        assert!(Rotation::parse("45rad").is_err());
    }

    #[test]
    fn parse_dimension() {
        let d = Dimension::parse("400x400").unwrap();
        assert_eq!(d.w, 400.0);
        assert_eq!(d.h, 400.0);
    }

    #[test]
    fn parse_color() {
        assert_eq!(
            Color::parse("#ff3366").unwrap(),
            Color::Hex("#ff3366".to_string())
        );
        assert_eq!(
            Color::parse("#1a1a2ecc").unwrap(),
            Color::Hex("#1a1a2ecc".to_string())
        );
        assert_eq!(Color::parse("none").unwrap(), Color::None);
        assert!(Color::parse("red").is_err());
    }

    #[test]
    fn parse_current_color() {
        // Canonical spelling and CSS-style case-insensitive variants all parse.
        assert_eq!(Color::parse("currentColor").unwrap(), Color::CurrentColor);
        assert_eq!(Color::parse("currentcolor").unwrap(), Color::CurrentColor);
        assert_eq!(Color::parse("CURRENTCOLOR").unwrap(), Color::CurrentColor);
        // Round-trips to the canonical spelling for the DSL/SVG.
        assert_eq!(Color::CurrentColor.to_string(), "currentColor");
    }

    #[test]
    fn parse_point_ref() {
        let p = PointRef::parse("stem.base").unwrap();
        assert_eq!(p.shape, "stem");
        assert_eq!(p.point, "base");
    }

    #[test]
    fn parse_segment_ref() {
        let s = SegmentRef::parse("stem.{base,mid}").unwrap();
        assert_eq!(s.shape, "stem");
        assert_eq!(s.p1, "base");
        assert_eq!(s.p2, "mid");
    }

    #[test]
    fn validate_ident_valid() {
        validate_ident("petal-1").unwrap();
        validate_ident("stem").unwrap();
        validate_ident("a").unwrap();
        validate_ident("brassDeep").unwrap();
        validate_ident("line_A").unwrap();
        validate_ident("R").unwrap();
    }

    #[test]
    fn validate_ident_invalid() {
        assert!(validate_ident("").is_err());
        assert!(validate_ident("1abc").is_err());
        assert!(validate_ident("-abc").is_err());
        assert!(validate_ident("a.b").is_err());
    }

    #[test]
    fn display_roundtrip() {
        assert_eq!(format!("{}", Dimension { w: 400.0, h: 300.0 }), "400x300");
        assert_eq!(format!("{}", Rotation(45.0)), "45deg");
        assert_eq!(format!("{}", RelativeSize(70.0)), "70%");
    }
}
