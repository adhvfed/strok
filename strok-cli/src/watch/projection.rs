use super::editing;
use std::collections::BTreeSet;
use strok_core::json::Json;
use strok_core::path_point::{path_data_to_svg_d, CurveMode, PathData};
use strok_core::shape::{Operation, Shape, Template};
use strok_core::types::PointMode;

pub(super) struct EditorProjection {
    pub(super) shapes: Json,
    pub(super) targets: Json,
}

pub(super) fn editor_projection(scene: &strok_core::scene::Scene) -> EditorProjection {
    let editable = editable_shape_names(scene);
    let shapes = editor_json(scene, &editable);
    let mut targets = Vec::new();
    collect_edit_targets(scene, &scene.nodes, &editable, &mut targets);
    EditorProjection {
        shapes,
        targets: Json::array(targets),
    }
}

fn editor_json(scene: &strok_core::scene::Scene, editable: &BTreeSet<String>) -> Json {
    let coord_space = (scene.document_size.w, scene.document_size.h);
    Json::array(scene.shapes.iter().filter_map(|shape| {
        if !editable.contains(&shape.name) {
            return None;
        }
        let data = shape.resolve(coord_space);
        if data.points.is_empty() {
            return None;
        }
        let starts = data.subpath_starts.clone();
        let points = data
            .points
            .iter()
            .enumerate()
            .map(|(index, point)| {
                let (mode, c1, c2, tension, arc) = match point.mode {
                    CurveMode::Sharp => ("sharp", None, None, None, None),
                    CurveMode::CatmullRom(tension) => ("smooth", None, None, Some(tension), None),
                    CurveMode::Arc {
                        rx,
                        ry,
                        sweep,
                        large,
                    } => ("arc", None, None, None, Some((rx, ry, sweep, large))),
                    CurveMode::Controls { c1, c2 } => ("controls", Some(c1), Some(c2), None, None),
                    CurveMode::ControlsRelative { c1, c2 } => {
                        let previous = previous_point(&data, index).unwrap_or(point);
                        (
                            "controls-relative",
                            Some((previous.x + c1.0, previous.y + c1.1)),
                            Some((point.x + c2.0, point.y + c2.1)),
                            None,
                            None,
                        )
                    }
                };
                Json::obj([
                    ("name", Json::str(point.name.clone())),
                    ("x", Json::num(point.x)),
                    ("y", Json::num(point.y)),
                    ("mode", Json::str(mode)),
                    ("start", Json::Bool(index == 0 || starts.contains(&index))),
                    (
                        "controlsEditable",
                        Json::Bool(control_source(shape, &point.name).is_some()),
                    ),
                    (
                        "canSymmetrize",
                        Json::Bool(can_symmetrize(shape, &data, index)),
                    ),
                    ("c1", pair_json(c1)),
                    ("c2", pair_json(c2)),
                    ("tension", tension.map(Json::num).unwrap_or(Json::Null)),
                    (
                        "arc",
                        arc.map(|(rx, ry, sweep, large)| {
                            Json::obj([
                                ("rx", Json::num(rx)),
                                ("ry", Json::num(ry)),
                                ("sweep", Json::Bool(sweep)),
                                ("large", Json::Bool(large)),
                            ])
                        })
                        .unwrap_or(Json::Null),
                    ),
                ])
            })
            .collect::<Vec<_>>();
        Some(Json::obj([
            ("name", Json::str(shape.name.clone())),
            ("d", Json::str(path_data_to_svg_d(&data, None))),
            ("closed", Json::Bool(data.closed)),
            ("points", Json::array(points)),
        ]))
    }))
}

fn collect_edit_targets(
    scene: &strok_core::scene::Scene,
    nodes: &[strok_core::scene::SceneNode],
    editable: &BTreeSet<String>,
    targets: &mut Vec<Json>,
) {
    use strok_core::scene::SceneNode;
    for node in nodes {
        match node {
            SceneNode::Place(place) if editable.contains(&place.shape_ref) => {
                let Some(transform) =
                    strok_core::resolve::placed_shape_transform(scene, &place.name)
                else {
                    continue;
                };
                targets.push(Json::obj([
                    ("name", Json::str(place.name.clone())),
                    ("shape", Json::str(place.shape_ref.clone())),
                    (
                        "transform",
                        Json::array(transform.into_iter().map(Json::num)),
                    ),
                ]));
            }
            SceneNode::Group(group) => {
                collect_edit_targets(scene, &group.children, editable, targets)
            }
            SceneNode::Boolean(boolean) => {
                collect_edit_targets(scene, &boolean.children, editable, targets)
            }
            SceneNode::Frame(frame) => {
                collect_edit_targets(scene, &frame.children, editable, targets)
            }
            SceneNode::Place(_) | SceneNode::Link(_) | SceneNode::Instance(_) => {}
        }
    }
}

fn editable_shape_names(scene: &strok_core::scene::Scene) -> BTreeSet<String> {
    let mut linked_shapes = Vec::new();
    collect_linked_shapes(&scene.nodes, &mut linked_shapes);
    scene
        .shapes
        .iter()
        .filter(|shape| {
            shape.template != Template::Text
                && !scene.imported_shape_names.contains(&shape.name)
                && !linked_shapes.contains(&shape.name)
                && !shape
                    .resolve((scene.document_size.w, scene.document_size.h))
                    .points
                    .is_empty()
        })
        .map(|shape| shape.name.clone())
        .collect()
}

pub(super) fn collect_linked_shapes(
    nodes: &[strok_core::scene::SceneNode],
    names: &mut Vec<String>,
) {
    use strok_core::scene::SceneNode;
    for node in nodes {
        match node {
            SceneNode::Link(link) => names.push(link.name.clone()),
            SceneNode::Group(group) => collect_linked_shapes(&group.children, names),
            SceneNode::Boolean(boolean) => collect_linked_shapes(&boolean.children, names),
            SceneNode::Frame(frame) => collect_linked_shapes(&frame.children, names),
            SceneNode::Place(_) | SceneNode::Instance(_) => {}
        }
    }
}

fn pair_json(value: Option<(f64, f64)>) -> Json {
    value
        .map(|(x, y)| Json::array([Json::num(x), Json::num(y)]))
        .unwrap_or(Json::Null)
}

pub(super) fn previous_point(
    data: &PathData,
    index: usize,
) -> Option<&strok_core::path_point::NamedPoint> {
    let (begin, end) = contour_bounds(data, index)?;
    if index > begin {
        data.points.get(index - 1)
    } else if data.closed {
        data.points.get(end.saturating_sub(1))
    } else {
        data.points.get(index)
    }
}

pub(super) fn previous_neighbor(
    data: &PathData,
    index: usize,
) -> Option<&strok_core::path_point::NamedPoint> {
    let (begin, end) = contour_bounds(data, index)?;
    if index > begin {
        data.points.get(index - 1)
    } else if data.closed {
        data.points.get(end.saturating_sub(1))
    } else {
        None
    }
}

pub(super) fn contour_bounds(data: &PathData, index: usize) -> Option<(usize, usize)> {
    if index >= data.points.len() {
        return None;
    }
    let begin = data
        .subpath_starts
        .iter()
        .copied()
        .take_while(|start| *start <= index)
        .last()
        .unwrap_or(0);
    let end = data
        .subpath_starts
        .iter()
        .copied()
        .find(|start| *start > index)
        .unwrap_or(data.points.len());
    Some((begin, end))
}

pub(super) fn control_source(shape: &Shape, point: &str) -> Option<usize> {
    shape.operations.iter().rposition(|operation| {
        matches!(
            operation,
            Operation::AddPoint {
                name,
                mode: Some(PointMode::Controls | PointMode::ControlsRelative),
                control_c1: Some(_),
                control_c2: Some(_),
                ..
            } if name == point
        )
    })
}

pub(super) fn addpoint_source(shape: &Shape, point: &str) -> Option<usize> {
    shape.operations.iter().rposition(
        |operation| matches!(operation, Operation::AddPoint { name, .. } if name == point),
    )
}

fn can_symmetrize(shape: &Shape, data: &PathData, index: usize) -> bool {
    let Some(point) = data.points.get(index) else {
        return false;
    };
    let Some(next) = editing::next_point(data, index) else {
        return false;
    };
    previous_neighbor(data, index).is_some()
        && addpoint_source(shape, &point.name).is_some()
        && addpoint_source(shape, &next.name).is_some()
}
