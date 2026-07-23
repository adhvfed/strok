// Several path-edit ops take many positional geometry args (point coords +
// modes + handles); grouping them into structs is a larger refactor deferred to
// the P2 path-editing work (C4/C5). Pre-existing signatures.
#![allow(clippy::too_many_arguments)]

use crate::attrs::Paint;
use crate::document::Document;
use crate::error::{Result, StrokError};
use crate::id;
use crate::node::{NodeKind, SceneNode};
use crate::ops::Operation;
use crate::path_point::{CurveMode, NamedPoint, PathData};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SculptAxis {
    Xy,
    Tangent,
    Normal,
}

impl Document {
    /// Move one named point in a DSL-defined path by (dx, dy).
    ///
    /// Returns the new (x, y) coordinates.
    pub fn move_point(
        &mut self,
        path: &str,
        point: &str,
        dx: f64,
        dy: f64,
        preserve_handles: bool,
    ) -> Result<(f64, f64)> {
        let (new_x, new_y) = {
            let pd = self.editable_path_data_mut(path)?;
            let idx = find_point_index(pd, point)?;

            if preserve_handles {
                shift_attached_handles(pd, idx, dx, dy);
            }

            let p = pd
                .points
                .get_mut(idx)
                .ok_or_else(|| StrokError::InvalidOperation("invalid point index".to_string()))?;
            p.x += dx;
            p.y += dy;
            (p.x, p.y)
        };

        self.history.push(Operation::SetAttr {
            node_id: path.to_string(),
            attr: format!("point:{}", point),
            value: format!("move {},{}", dx, dy),
        });
        Ok((new_x, new_y))
    }

    /// Pull a point and neighboring points with distance-based falloff.
    ///
    /// Returns a list of moved points with their new coordinates.
    pub fn pull_point(
        &mut self,
        path: &str,
        point: &str,
        dx: f64,
        dy: f64,
        radius: usize,
        falloff: f64,
        preserve_handles: bool,
    ) -> Result<Vec<(String, f64, f64)>> {
        let moved = {
            let pd = self.editable_path_data_mut(path)?;
            let center = find_point_index(pd, point)?;
            let n = pd.points.len();
            let safe_falloff = if falloff <= 0.0 { 1.0 } else { falloff };
            let mut moved = Vec::new();

            for i in 0..n {
                let linear = i.abs_diff(center);
                let dist = if pd.closed {
                    linear.min(n - linear)
                } else {
                    linear
                };
                if dist > radius {
                    continue;
                }

                let weight = if radius == 0 {
                    1.0
                } else {
                    let base = (radius + 1 - dist) as f64 / (radius + 1) as f64;
                    base.powf(safe_falloff)
                };
                let mdx = dx * weight;
                let mdy = dy * weight;

                if preserve_handles {
                    shift_attached_handles(pd, i, mdx, mdy);
                }

                if let Some(p) = pd.points.get_mut(i) {
                    p.x += mdx;
                    p.y += mdy;
                    moved.push((p.name.clone(), p.x, p.y));
                }
            }
            moved
        };

        self.history.push(Operation::SetAttr {
            node_id: path.to_string(),
            attr: format!("point:{}", point),
            value: format!("pull {},{} r={} falloff={}", dx, dy, radius, falloff),
        });
        Ok(moved)
    }

    /// Sculpt a path by pulling points around an absolute coordinate.
    ///
    /// Points inside `radius` are moved by a falloff-scaled delta.
    pub fn sculpt_path(
        &mut self,
        path: &str,
        at_x: f64,
        at_y: f64,
        dx: f64,
        dy: f64,
        radius: f64,
        falloff: f64,
        preserve_handles: bool,
    ) -> Result<Vec<(String, f64, f64)>> {
        self.sculpt_path_with_options(
            path,
            at_x,
            at_y,
            dx,
            dy,
            radius,
            falloff,
            SculptAxis::Xy,
            false,
            preserve_handles,
        )
    }

    /// Sculpt a path with advanced direction controls.
    ///
    /// - `axis = Xy`: use the supplied `(dx, dy)` directly.
    /// - `axis = Tangent|Normal`: project `(dx, dy)` onto each point's local axis.
    /// - `lock_endpoints`: do not move first/last points on open paths.
    pub fn sculpt_path_with_options(
        &mut self,
        path: &str,
        at_x: f64,
        at_y: f64,
        dx: f64,
        dy: f64,
        radius: f64,
        falloff: f64,
        axis: SculptAxis,
        lock_endpoints: bool,
        preserve_handles: bool,
    ) -> Result<Vec<(String, f64, f64)>> {
        if radius <= 0.0 {
            return Err(StrokError::InvalidOperation(format!(
                "sculpt radius must be > 0, got {}",
                radius
            )));
        }

        let moved = {
            let pd = self.editable_path_data_mut(path)?;
            let n = pd.points.len();
            let safe_falloff = if falloff <= 0.0 { 1.0 } else { falloff };
            let mut moved = Vec::new();
            let original: Vec<(f64, f64)> = pd.points.iter().map(|p| (p.x, p.y)).collect();

            // Compute all deltas first from the original geometry.
            let deltas: Vec<(usize, f64, f64)> = pd
                .points
                .iter()
                .enumerate()
                .filter_map(|(i, p)| {
                    if lock_endpoints && !pd.closed && (i == 0 || i + 1 == n) {
                        return None;
                    }
                    let ddx = p.x - at_x;
                    let ddy = p.y - at_y;
                    let dist = (ddx * ddx + ddy * ddy).sqrt();
                    if dist > radius {
                        return None;
                    }
                    let weight = (1.0 - (dist / radius)).max(0.0).powf(safe_falloff);
                    let (mdx, mdy) = match axis {
                        SculptAxis::Xy => (dx * weight, dy * weight),
                        SculptAxis::Tangent | SculptAxis::Normal => {
                            let (axis_x, axis_y) = match axis {
                                SculptAxis::Tangent => point_tangent(&original, pd.closed, i),
                                SculptAxis::Normal => {
                                    let (tx, ty) = point_tangent(&original, pd.closed, i);
                                    (-ty, tx)
                                }
                                _ => unreachable!(),
                            };
                            let projected = dx * axis_x + dy * axis_y;
                            (axis_x * projected * weight, axis_y * projected * weight)
                        }
                    };
                    Some((i, mdx, mdy))
                })
                .collect();

            for (i, mdx, mdy) in deltas {
                if preserve_handles {
                    shift_attached_handles(pd, i, mdx, mdy);
                }
                if let Some(p) = pd.points.get_mut(i) {
                    p.x += mdx;
                    p.y += mdy;
                    moved.push((p.name.clone(), p.x, p.y));
                }
            }
            moved
        };

        self.history.push(Operation::SetAttr {
            node_id: path.to_string(),
            attr: "sculpt".to_string(),
            value: format!(
                "at={},{} delta={},{} radius={} falloff={} axis={:?} lock_endpoints={}",
                at_x, at_y, dx, dy, radius, falloff, axis, lock_endpoints
            ),
        });
        Ok(moved)
    }

    /// Create a branch path from an existing point in a source path.
    ///
    /// The new path is appended as a sibling under the source path's parent.
    pub fn branch_from_point(
        &mut self,
        source_path: &str,
        from_point: &str,
        new_id: &str,
        length: f64,
        angle_deg: f64,
        bend: f64,
        stroke_width: Option<f64>,
    ) -> Result<String> {
        if length <= 0.0 {
            return Err(StrokError::InvalidOperation(format!(
                "branch length must be > 0, got {}",
                length
            )));
        }
        id::validate_id(new_id, &self.id_index)?;

        let source_nid = self.resolve_id(source_path)?;
        let (parent_id, anchor_x, anchor_y, dir_x, dir_y, src_stroke, src_sw) = {
            let node = self.arena.get(source_nid)?;
            if node.kind != NodeKind::Path {
                return Err(StrokError::InvalidOperation(format!(
                    "'{}' is not a path",
                    source_path
                )));
            }
            let pd = node.attrs.path_data.as_ref().ok_or_else(|| {
                StrokError::InvalidOperation(format!("'{}' has no editable path data", source_path))
            })?;
            let idx = find_point_index(pd, from_point)?;
            let p = pd.points.get(idx).ok_or_else(|| {
                StrokError::InvalidOperation("invalid source point index".to_string())
            })?;

            let coords: Vec<(f64, f64)> = pd.points.iter().map(|pt| (pt.x, pt.y)).collect();
            let (tx, ty) = point_tangent(&coords, pd.closed, idx);
            let parent_id = node.parent.ok_or_else(|| {
                StrokError::InvalidOperation(format!("'{}' has no parent", source_path))
            })?;
            (
                parent_id,
                p.x,
                p.y,
                tx,
                ty,
                node.attrs.stroke.clone(),
                node.attrs.stroke_width,
            )
        };

        let theta = angle_deg.to_radians();
        let branch_dx = dir_x * theta.cos() - dir_y * theta.sin();
        let branch_dy = dir_x * theta.sin() + dir_y * theta.cos();
        let (branch_dx, branch_dy) = normalize_or_default(branch_dx, branch_dy, 0.0, -1.0);
        let perp_x = -branch_dy;
        let perp_y = branch_dx;

        let bend_amt = bend * length;
        let p0 = (anchor_x, anchor_y);
        let p1 = (
            anchor_x + branch_dx * (length * 0.34) + perp_x * (bend_amt * 0.30),
            anchor_y + branch_dy * (length * 0.34) + perp_y * (bend_amt * 0.30),
        );
        let p2 = (
            anchor_x + branch_dx * (length * 0.7) + perp_x * (bend_amt * 0.55),
            anchor_y + branch_dy * (length * 0.7) + perp_y * (bend_amt * 0.55),
        );
        let p3 = (anchor_x + branch_dx * length, anchor_y + branch_dy * length);

        let mut node = SceneNode::new(new_id.to_string(), NodeKind::Path);
        node.attrs.fill = Some(Paint::None);
        node.attrs.stroke = src_stroke.or(Some(Paint::Color("#2f7139".to_string())));
        node.attrs.stroke_width =
            Some(stroke_width.unwrap_or_else(|| src_sw.map(|w| (w * 0.6).max(1.0)).unwrap_or(4.0)));
        node.attrs
            .extra
            .insert("stroke-linecap".to_string(), "round".to_string());
        node.attrs
            .extra
            .insert("stroke-linejoin".to_string(), "round".to_string());
        node.attrs.path_data = Some(PathData {
            coord_space: (self.width, self.height),
            points: vec![
                NamedPoint {
                    name: "p0".to_string(),
                    x: p0.0,
                    y: p0.1,
                    mode: CurveMode::Sharp,
                },
                NamedPoint {
                    name: "p1".to_string(),
                    x: p1.0,
                    y: p1.1,
                    mode: CurveMode::CatmullRom(-0.1),
                },
                NamedPoint {
                    name: "p2".to_string(),
                    x: p2.0,
                    y: p2.1,
                    mode: CurveMode::CatmullRom(-0.3),
                },
                NamedPoint {
                    name: "p3".to_string(),
                    x: p3.0,
                    y: p3.1,
                    mode: CurveMode::CatmullRom(0.0),
                },
            ],
            closed: false,
            subpath_starts: Vec::new(),
        });

        let new_nid = self.arena.alloc(node);
        self.id_index.insert(new_id.to_string(), new_nid);
        self.arena.append_child(parent_id, new_nid)?;

        let parent_name = self
            .node_path(parent_id)
            .unwrap_or_else(|_| "root".to_string());
        self.history.push(Operation::Append {
            parent: parent_name,
            node_id: new_id.to_string(),
        });
        Ok(new_id.to_string())
    }

    /// Create several branch paths from one source point with fan-out/spread.
    pub fn sprout_from_point(
        &mut self,
        source_path: &str,
        from_point: &str,
        id_prefix: &str,
        count: usize,
        length: f64,
        angle_deg: f64,
        spread_deg: f64,
        bend: f64,
        jitter: f64,
        stroke_width: Option<f64>,
    ) -> Result<Vec<String>> {
        if count == 0 {
            return Err(StrokError::InvalidOperation(
                "sprout count must be >= 1".to_string(),
            ));
        }
        if length <= 0.0 {
            return Err(StrokError::InvalidOperation(format!(
                "sprout length must be > 0, got {}",
                length
            )));
        }

        let jitter_amount = jitter.max(0.0);
        // Hash the source point name for a position-dependent seed offset.
        let seed_offset: u64 = from_point
            .bytes()
            .fold(0u64, |acc, b| acc.wrapping_mul(31).wrapping_add(b as u64));
        let mut created = Vec::with_capacity(count);
        for i in 0..count {
            let id = format!("{}-{}", id_prefix, i + 1);

            let spread_u = if count <= 1 {
                0.0
            } else {
                i as f64 / (count - 1) as f64 - 0.5
            };
            let base_angle = angle_deg + spread_u * spread_deg;

            let noise_a = signed_noise(i as u64, 101_u64.wrapping_add(seed_offset));
            let noise_l = signed_noise(i as u64, 211_u64.wrapping_add(seed_offset));
            let noise_b = signed_noise(i as u64, 307_u64.wrapping_add(seed_offset));
            let noise_w = signed_noise(i as u64, 401_u64.wrapping_add(seed_offset));

            let branch_angle = base_angle + noise_a * spread_deg * 0.25 * jitter_amount;
            let branch_length = (length * (1.0 + noise_l * 0.2 * jitter_amount)).max(1.0);
            let branch_bend = bend + noise_b * 0.35 * jitter_amount;
            let branch_width =
                stroke_width.map(|sw| (sw * (1.0 + noise_w * 0.15 * jitter_amount)).max(0.1));

            self.branch_from_point(
                source_path,
                from_point,
                &id,
                branch_length,
                branch_angle,
                branch_bend,
                branch_width,
            )?;
            created.push(id);
        }

        Ok(created)
    }

    /// Return the absolute coordinates of a named point in a DSL path.
    pub fn point_position(&self, path: &str, point: &str) -> Result<(f64, f64)> {
        let pd = self.editable_path_data(path)?;
        let idx = find_point_index(pd, point)?;
        let p = pd
            .points
            .get(idx)
            .ok_or_else(|| StrokError::InvalidOperation("invalid point index".to_string()))?;
        Ok((p.x, p.y))
    }

    /// Return an interpolated position along a specific path segment.
    pub fn segment_position(
        &self,
        path: &str,
        from_point: &str,
        to_point: &str,
        t: f64,
    ) -> Result<(f64, f64)> {
        if !(0.0..=1.0).contains(&t) {
            return Err(StrokError::InvalidOperation(format!(
                "t must be within [0,1], got {}",
                t
            )));
        }

        let pd = self.editable_path_data(path)?;
        let n = pd.points.len();
        if n < 2 {
            return Err(StrokError::InvalidOperation(format!(
                "'{}' requires at least 2 points",
                path
            )));
        }

        let from_idx = find_point_index(pd, from_point)?;
        let to_idx = find_point_index(pd, to_point)?;
        let adjacent_forward = from_idx + 1 == to_idx;
        let adjacent_wrapped = pd.closed && from_idx == n - 1 && to_idx == 0;
        if !adjacent_forward && !adjacent_wrapped {
            return Err(StrokError::InvalidOperation(format!(
                "segment '{}' -> '{}' is not consecutive in '{}'",
                from_point, to_point, path
            )));
        }

        let from = pd.points.get(from_idx).ok_or_else(|| {
            StrokError::InvalidOperation("invalid segment start index".to_string())
        })?;
        let to = pd
            .points
            .get(to_idx)
            .ok_or_else(|| StrokError::InvalidOperation("invalid segment end index".to_string()))?;

        let x = from.x + (to.x - from.x) * t;
        let y = from.y + (to.y - from.y) * t;
        Ok((x, y))
    }

    /// Insert a new named point after an existing point.
    pub fn insert_point_after(
        &mut self,
        path: &str,
        after_point: &str,
        new_point: &str,
        x: f64,
        y: f64,
        mode: CurveMode,
    ) -> Result<()> {
        {
            let pd = self.editable_path_data_mut(path)?;
            if pd.points.iter().any(|p| p.name == new_point) {
                return Err(StrokError::InvalidOperation(format!(
                    "point '{}' already exists in '{}'",
                    new_point, path
                )));
            }
            let idx = find_point_index(pd, after_point)?;
            pd.points.insert(
                idx + 1,
                NamedPoint {
                    name: new_point.to_string(),
                    x,
                    y,
                    mode,
                },
            );
        }

        self.history.push(Operation::SetAttr {
            node_id: path.to_string(),
            attr: format!("point:{}", new_point),
            value: format!("insert-after {}", after_point),
        });
        Ok(())
    }

    /// Delete a named point from a path.
    ///
    /// `reconnect_mode` controls the resulting segment style:
    /// - `None`: preserve the surviving "to" point's current mode.
    /// - `Some(mode)`: force that mode on the surviving "to" point.
    pub fn delete_point(
        &mut self,
        path: &str,
        point: &str,
        reconnect_mode: Option<CurveMode>,
    ) -> Result<()> {
        {
            let pd = self.editable_path_data_mut(path)?;
            let n = pd.points.len();
            let min_points = if pd.closed { 3 } else { 2 };
            if n <= min_points {
                return Err(StrokError::InvalidOperation(format!(
                    "cannot delete point from '{}': path requires at least {} points",
                    path, min_points
                )));
            }

            let idx = find_point_index(pd, point)?;
            pd.points.remove(idx);

            if let Some(mode) = reconnect_mode {
                let maybe_to_idx = if pd.closed {
                    if pd.points.is_empty() {
                        None
                    } else if idx >= pd.points.len() {
                        Some(0)
                    } else {
                        Some(idx)
                    }
                } else if idx == 0 || idx >= pd.points.len() {
                    None
                } else {
                    Some(idx)
                };

                if let Some(to_idx) = maybe_to_idx {
                    if let Some(p) = pd.points.get_mut(to_idx) {
                        p.mode = mode;
                    }
                }
            }
        }

        self.history.push(Operation::SetAttr {
            node_id: path.to_string(),
            attr: format!("point:{}", point),
            value: "delete".to_string(),
        });
        Ok(())
    }

    /// Rename a point in a path.
    pub fn rename_point(&mut self, path: &str, old_name: &str, new_name: &str) -> Result<()> {
        {
            let pd = self.editable_path_data_mut(path)?;
            if old_name == new_name {
                return Ok(());
            }
            if pd.points.iter().any(|p| p.name == new_name) {
                return Err(StrokError::InvalidOperation(format!(
                    "point '{}' already exists in '{}'",
                    new_name, path
                )));
            }
            let idx = find_point_index(pd, old_name)?;
            let p = pd
                .points
                .get_mut(idx)
                .ok_or_else(|| StrokError::InvalidOperation("invalid point index".to_string()))?;
            p.name = new_name.to_string();
        }

        self.history.push(Operation::SetAttr {
            node_id: path.to_string(),
            attr: format!("point:{}->{}", old_name, new_name),
            value: "rename".to_string(),
        });
        Ok(())
    }

    /// Set the curve mode on a named point.
    pub fn set_point_mode(&mut self, path: &str, point: &str, mode: CurveMode) -> Result<()> {
        {
            let pd = self.editable_path_data_mut(path)?;
            let idx = find_point_index(pd, point)?;
            let p = pd
                .points
                .get_mut(idx)
                .ok_or_else(|| StrokError::InvalidOperation("invalid point index".to_string()))?;
            p.mode = mode;
        }

        self.history.push(Operation::SetAttr {
            node_id: path.to_string(),
            attr: format!("point:{}", point),
            value: "set-mode".to_string(),
        });
        Ok(())
    }

    /// Split a segment between consecutive points by inserting a new point.
    ///
    /// Returns the inserted point coordinates.
    pub fn split_segment(
        &mut self,
        path: &str,
        from_point: &str,
        to_point: &str,
        new_point: &str,
        t: f64,
    ) -> Result<(f64, f64)> {
        if !(0.0..=1.0).contains(&t) {
            return Err(StrokError::InvalidOperation(format!(
                "split t must be between 0 and 1, got {}",
                t
            )));
        }

        let (x, y) = {
            let pd = self.editable_path_data_mut(path)?;
            if pd.points.iter().any(|p| p.name == new_point) {
                return Err(StrokError::InvalidOperation(format!(
                    "point '{}' already exists in '{}'",
                    new_point, path
                )));
            }

            let from_idx = find_point_index(pd, from_point)?;
            let to_idx = find_point_index(pd, to_point)?;
            let n = pd.points.len();

            let adjacent_forward = from_idx + 1 == to_idx;
            let adjacent_wrapped = pd.closed && from_idx == n.saturating_sub(1) && to_idx == 0;
            if !adjacent_forward && !adjacent_wrapped {
                return Err(StrokError::InvalidOperation(format!(
                    "points '{}' and '{}' are not consecutive in '{}'",
                    from_point, to_point, path
                )));
            }

            let from = pd.points.get(from_idx).ok_or_else(|| {
                StrokError::InvalidOperation("invalid segment start index".to_string())
            })?;
            let to = pd.points.get(to_idx).ok_or_else(|| {
                StrokError::InvalidOperation("invalid segment end index".to_string())
            })?;

            let x = from.x + (to.x - from.x) * t;
            let y = from.y + (to.y - from.y) * t;
            let insert_idx = if adjacent_wrapped { n } else { to_idx };
            pd.points.insert(
                insert_idx,
                NamedPoint {
                    name: new_point.to_string(),
                    x,
                    y,
                    mode: CurveMode::Sharp,
                },
            );
            (x, y)
        };

        self.history.push(Operation::SetAttr {
            node_id: path.to_string(),
            attr: format!("point:{}", new_point),
            value: format!("split {}->{} t={}", from_point, to_point, t),
        });
        Ok((x, y))
    }

    pub(crate) fn editable_path_data(&self, path: &str) -> Result<&PathData> {
        let nid = self.resolve_id(path)?;
        let node = self.arena.get(nid)?;
        if node.kind != NodeKind::Path {
            return Err(StrokError::InvalidOperation(format!(
                "'{}' is not a path element",
                path
            )));
        }
        match (node.attrs.path_data.as_ref(), node.attrs.d.as_ref()) {
            (Some(pd), _) => Ok(pd),
            (None, Some(_)) => Err(StrokError::InvalidOperation(format!(
                "'{}' stores raw SVG path data (`d`) and is not point-editable",
                path
            ))),
            (None, None) => Err(StrokError::InvalidOperation(format!(
                "'{}' has no path data",
                path
            ))),
        }
    }

    pub(crate) fn editable_path_data_mut(&mut self, path: &str) -> Result<&mut PathData> {
        let nid = self.resolve_id(path)?;
        let node = self.arena.get_mut(nid)?;
        if node.kind != NodeKind::Path {
            return Err(StrokError::InvalidOperation(format!(
                "'{}' is not a path element",
                path
            )));
        }
        match (node.attrs.path_data.as_mut(), node.attrs.d.as_ref()) {
            (Some(pd), _) => Ok(pd),
            (None, Some(_)) => Err(StrokError::InvalidOperation(format!(
                "'{}' stores raw SVG path data (`d`) and is not point-editable",
                path
            ))),
            (None, None) => Err(StrokError::InvalidOperation(format!(
                "'{}' has no path data",
                path
            ))),
        }
    }
}

fn find_point_index(pd: &PathData, point: &str) -> Result<usize> {
    let idx = pd
        .points
        .iter()
        .position(|p| p.name == point)
        .ok_or_else(|| {
            let names: Vec<String> = pd.points.iter().map(|p| p.name.clone()).collect();
            StrokError::InvalidOperation(format!(
                "point '{}' not found. Available points: {:?}",
                point, names
            ))
        })?;
    Ok(idx)
}

fn shift_attached_handles(pd: &mut PathData, idx: usize, dx: f64, dy: f64) {
    let n = pd.points.len();
    if n == 0 || idx >= n {
        return;
    }

    // Incoming handle attached to the moved point for segment prev -> current.
    if let Some(curr) = pd.points.get_mut(idx) {
        if let CurveMode::Controls { c2, .. } = &mut curr.mode {
            c2.0 += dx;
            c2.1 += dy;
        }
        // ControlsRelative: handles are relative offsets from their endpoint,
        // so they move implicitly when the point moves — no shift needed.
    }

    // Outgoing handle attached to the moved point for segment current -> next.
    let next_idx = if idx + 1 < n {
        Some(idx + 1)
    } else if pd.closed {
        Some(0)
    } else {
        None
    };
    if let Some(next_idx) = next_idx {
        if let Some(next) = pd.points.get_mut(next_idx) {
            if let CurveMode::Controls { c1, .. } = &mut next.mode {
                c1.0 += dx;
                c1.1 += dy;
            }
            // ControlsRelative: handles are relative offsets, no shift needed.
        }
    }
}

fn point_tangent(points: &[(f64, f64)], closed: bool, i: usize) -> (f64, f64) {
    let n = points.len();
    if n == 0 || i >= n {
        return (0.0, -1.0);
    }

    let prev_i = if i > 0 {
        i - 1
    } else if closed {
        n - 1
    } else {
        i
    };
    let next_i = if i + 1 < n {
        i + 1
    } else if closed {
        0
    } else {
        i
    };

    let (px, py) = points[prev_i];
    let (nx, ny) = points[next_i];
    normalize_or_default(nx - px, ny - py, 0.0, -1.0)
}

fn signed_noise(i: u64, salt: u64) -> f64 {
    let mut x = i.wrapping_mul(0x9e37_79b9_7f4a_7c15).wrapping_add(salt);
    x ^= x >> 33;
    x = x.wrapping_mul(0xff51_afd7_ed55_8ccd);
    x ^= x >> 33;
    x = x.wrapping_mul(0xc4ce_b9fe_1a85_ec53);
    x ^= x >> 33;
    let unit = (x as f64) / (u64::MAX as f64);
    unit * 2.0 - 1.0
}

fn normalize_or_default(x: f64, y: f64, default_x: f64, default_y: f64) -> (f64, f64) {
    let len = (x * x + y * y).sqrt();
    if len <= 1e-9 {
        (default_x, default_y)
    } else {
        (x / len, y / len)
    }
}
