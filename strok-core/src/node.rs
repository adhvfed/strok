use crate::attrs::Attrs;
use serde::{Deserialize, Serialize};

/// Index into the arena.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct NodeId(pub u32);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum NodeKind {
    Root,
    Group,
    Rect,
    Ellipse,
    Circle,
    Path,
    Line,
    Polygon,
    Polyline,
    Text,
    Image,
}

impl NodeKind {
    pub fn from_tag(tag: &str) -> Option<Self> {
        match tag {
            "svg" => Some(Self::Root),
            "g" => Some(Self::Group),
            "rect" => Some(Self::Rect),
            "ellipse" => Some(Self::Ellipse),
            "circle" => Some(Self::Circle),
            "path" => Some(Self::Path),
            "line" => Some(Self::Line),
            "polygon" => Some(Self::Polygon),
            "polyline" => Some(Self::Polyline),
            "text" => Some(Self::Text),
            "image" => Some(Self::Image),
            _ => None,
        }
    }

    pub fn tag_name(&self) -> &'static str {
        match self {
            Self::Root => "svg",
            Self::Group => "g",
            Self::Rect => "rect",
            Self::Ellipse => "ellipse",
            Self::Circle => "circle",
            Self::Path => "path",
            Self::Line => "line",
            Self::Polygon => "polygon",
            Self::Polyline => "polyline",
            Self::Text => "text",
            Self::Image => "image",
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SceneNode {
    pub id: String,
    pub kind: NodeKind,
    pub attrs: Attrs,
    pub children: Vec<NodeId>,
    pub parent: Option<NodeId>,
    /// If this node was instantiated from a shape, the shape name.
    #[serde(skip)]
    pub shape_ref: Option<String>,
}

impl SceneNode {
    pub fn new(id: String, kind: NodeKind) -> Self {
        Self {
            id,
            kind,
            attrs: Attrs::default(),
            children: Vec::new(),
            parent: None,
            shape_ref: None,
        }
    }
}
