use std::collections::HashMap;
use std::path::Path;

use serde::{Deserialize, Serialize};

use crate::error::{Result, StrokError};
use crate::id;
use crate::node::{NodeId, NodeKind, SceneNode};
use crate::ops::{Operation, OperationLog};
use crate::scene::Scene;
use crate::tree::Arena;

const MAGIC: &[u8; 4] = b"STRK";
const VERSION: u32 = 1;

#[derive(Debug, Serialize, Deserialize)]
struct StrokFile {
    magic: [u8; 4],
    version: u32,
    document: Document,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Document {
    pub width: f64,
    pub height: f64,
    pub arena: Arena,
    pub root_id: NodeId,
    pub id_index: HashMap<String, NodeId>,
    pub history: OperationLog,
    /// v3 scene graph.
    #[serde(skip)]
    pub scene: Option<Scene>,
}

impl Document {
    pub fn new(width: f64, height: f64) -> Self {
        let mut arena = Arena::new();
        let root = SceneNode::new("root".to_string(), NodeKind::Root);
        let root_id = arena.alloc(root);
        let mut id_index = HashMap::new();
        id_index.insert("root".to_string(), root_id);

        let mut doc = Self {
            width,
            height,
            arena,
            root_id,
            id_index,
            history: OperationLog::new(),
            scene: None,
        };
        doc.history.push(Operation::New { width, height });
        doc
    }

    /// Create a Document from a v3 Scene.
    pub fn from_scene(scene: Scene) -> Self {
        let width = scene.document_size.w;
        let height = scene.document_size.h;
        let mut doc = Self::new(width, height);
        doc.scene = Some(scene);
        doc
    }

    /// Resolve a name or path query to a NodeId.
    pub fn resolve_id(&self, query: &str) -> Result<NodeId> {
        if let Some(&nid) = self.id_index.get(query) {
            return Ok(nid);
        }

        if query.contains('/') {
            return self.resolve_path_query(query);
        }

        let mut matches = Vec::new();
        for (id, &nid) in &self.id_index {
            let leaf = id.rsplit('/').next().unwrap_or(id);
            if leaf == query || *id == query {
                matches.push(nid);
            }
        }

        if matches.is_empty() {
            self.find_nodes_by_name(query, self.root_id, &mut matches);
        }

        match matches.len() {
            0 => Err(StrokError::IdNotFound(query.to_string())),
            1 => Ok(matches[0]),
            _ => {
                let candidates: Vec<String> = matches
                    .iter()
                    .filter_map(|&nid| self.node_path(nid).ok())
                    .collect();
                Err(StrokError::AmbiguousName {
                    name: query.to_string(),
                    candidates,
                })
            }
        }
    }

    fn resolve_path_query(&self, query: &str) -> Result<NodeId> {
        let segments: Vec<&str> = query.split('/').collect();
        let mut current = self.root_id;

        for seg in &segments {
            let node = self.arena.get(current)?;
            let mut found = false;
            for &child_id in &node.children {
                let child = self.arena.get(child_id)?;
                if child.id == *seg {
                    current = child_id;
                    found = true;
                    break;
                }
            }
            if !found {
                return Err(StrokError::IdNotFound(query.to_string()));
            }
        }

        Ok(current)
    }

    fn find_nodes_by_name(&self, name: &str, node_id: NodeId, results: &mut Vec<NodeId>) {
        if let Ok(node) = self.arena.get(node_id) {
            if node.id == name {
                results.push(node_id);
            }
            for &child_id in &node.children {
                self.find_nodes_by_name(name, child_id, results);
            }
        }
    }

    pub fn node_path(&self, node_id: NodeId) -> Result<String> {
        let mut segments = Vec::new();
        let mut current = node_id;
        loop {
            let node = self.arena.get(current)?;
            if node.kind == NodeKind::Root {
                break;
            }
            segments.push(node.id.clone());
            match node.parent {
                Some(pid) => current = pid,
                None => break,
            }
        }
        segments.reverse();
        Ok(segments.join("/"))
    }

    pub fn find_by_id(&self, name: &str) -> Result<&SceneNode> {
        let nid = self.resolve_id(name)?;
        self.arena.get(nid)
    }

    fn register_node(&mut self, node_id: NodeId) -> Result<()> {
        let id = self.arena.get(node_id)?.id.clone();
        self.id_index.insert(id, node_id);
        let children: Vec<NodeId> = self.arena.get(node_id)?.children.clone();
        for child_id in children {
            self.register_node(child_id)?;
        }
        Ok(())
    }

    fn unregister_node(&mut self, node_id: NodeId) -> Result<()> {
        let children: Vec<NodeId> = self.arena.get(node_id)?.children.clone();
        for child_id in children {
            self.unregister_node(child_id)?;
        }
        let id = self.arena.get(node_id)?.id.clone();
        self.id_index.remove(&id);
        Ok(())
    }

    pub fn alloc_parsed_tree(&mut self, nodes: &[SceneNode]) -> Result<NodeId> {
        self.alloc_parsed_tree_scoped(nodes, None)
    }

    pub fn alloc_parsed_tree_scoped(
        &mut self,
        nodes: &[SceneNode],
        scope: Option<&str>,
    ) -> Result<NodeId> {
        if nodes.is_empty() {
            return Err(StrokError::ParseError("empty node list".to_string()));
        }

        for (i, node) in nodes.iter().enumerate() {
            if !node.id.is_empty() {
                let index_key = match scope {
                    Some(s) if i != 0 => format!("{}/{}", s, node.id),
                    _ => node.id.clone(),
                };
                id::validate_id(&index_key, &self.id_index)?;
            }
        }

        let mut index_map: Vec<Option<NodeId>> = vec![None; nodes.len()];

        for (i, parsed) in nodes.iter().enumerate() {
            let node_id_str = if parsed.id.is_empty() {
                id::generate_id(&self.id_index)
            } else {
                parsed.id.clone()
            };

            let mut node = SceneNode::new(node_id_str, parsed.kind.clone());
            node.attrs = parsed.attrs.clone();
            node.shape_ref = parsed.shape_ref.clone();
            node.children = Vec::new();

            let nid = self.arena.alloc(node);
            let id_str = self.arena.get(nid)?.id.clone();

            let index_key = match scope {
                Some(s) if i != 0 => format!("{}/{}", s, id_str),
                _ => id_str,
            };
            self.id_index.insert(index_key, nid);
            index_map[i] = Some(nid);
        }

        for (i, parsed) in nodes.iter().enumerate() {
            // Invariant: the loop above set `index_map[i] = Some(_)` for every i.
            let parent_nid = index_map[i].ok_or_else(|| {
                StrokError::InvalidOperation("internal: unallocated parent node".to_string())
            })?;
            for &child_placeholder in &parsed.children {
                let child_idx = child_placeholder.0 as usize;
                let child_nid = index_map.get(child_idx).copied().flatten().ok_or_else(|| {
                    StrokError::ParseError(format!("child placeholder {} out of range", child_idx))
                })?;
                self.arena.get_mut(child_nid)?.parent = Some(parent_nid);
                self.arena.get_mut(parent_nid)?.children.push(child_nid);
            }
        }

        index_map[0].ok_or_else(|| {
            StrokError::InvalidOperation("internal: unallocated root node".to_string())
        })
    }

    pub fn append_svg(&mut self, parent_name: &str, svg: &str) -> Result<String> {
        let nodes = crate::parse::parse_snippet(svg)?;
        let parent_id = self.resolve_id(parent_name)?;
        let node_id = self.alloc_parsed_tree(&nodes)?;
        self.arena.append_child(parent_id, node_id)?;
        let final_id = self.arena.get(node_id)?.id.clone();
        self.history.push(Operation::Append {
            parent: parent_name.to_string(),
            node_id: final_id.clone(),
        });
        Ok(final_id)
    }

    pub fn prepend_svg(&mut self, parent_name: &str, svg: &str) -> Result<String> {
        let nodes = crate::parse::parse_snippet(svg)?;
        let parent_id = self.resolve_id(parent_name)?;
        let node_id = self.alloc_parsed_tree(&nodes)?;
        self.arena.prepend_child(parent_id, node_id)?;
        let final_id = self.arena.get(node_id)?.id.clone();
        self.history.push(Operation::Prepend {
            parent: parent_name.to_string(),
            node_id: final_id.clone(),
        });
        Ok(final_id)
    }

    pub fn insert_svg(&mut self, parent_name: &str, index: usize, svg: &str) -> Result<String> {
        let nodes = crate::parse::parse_snippet(svg)?;
        let parent_id = self.resolve_id(parent_name)?;
        let node_id = self.alloc_parsed_tree(&nodes)?;
        self.arena.insert_child(parent_id, index, node_id)?;
        let final_id = self.arena.get(node_id)?.id.clone();
        self.history.push(Operation::Insert {
            parent: parent_name.to_string(),
            index,
            node_id: final_id.clone(),
        });
        Ok(final_id)
    }

    pub fn replace_svg(&mut self, name: &str, svg: &str) -> Result<String> {
        if name == "root" {
            return Err(StrokError::InvalidOperation(
                "cannot replace root node".to_string(),
            ));
        }
        let old_nid = self.resolve_id(name)?;
        let parent_id = self
            .arena
            .get(old_nid)?
            .parent
            .ok_or_else(|| StrokError::InvalidOperation("node has no parent".to_string()))?;

        let index = self
            .arena
            .get(parent_id)?
            .children
            .iter()
            .position(|&id| id == old_nid)
            .ok_or_else(|| {
                StrokError::InvalidOperation("node not found among parent's children".to_string())
            })?;

        self.unregister_node(old_nid)?;
        self.arena.detach(old_nid)?;
        self.arena.remove_subtree(old_nid)?;

        let mut nodes = crate::parse::parse_snippet(svg)?;
        if nodes[0].id.is_empty() {
            nodes[0].id = name.to_string();
        }

        let new_nid = self.alloc_parsed_tree(&nodes)?;
        self.arena.insert_child(parent_id, index, new_nid)?;
        let final_id = self.arena.get(new_nid)?.id.clone();
        self.history.push(Operation::Replace {
            old_id: name.to_string(),
            new_id: final_id.clone(),
        });
        Ok(final_id)
    }

    pub fn remove(&mut self, name: &str) -> Result<()> {
        if name == "root" {
            return Err(StrokError::InvalidOperation(
                "cannot remove root node".to_string(),
            ));
        }
        let node_id = self.resolve_id(name)?;
        self.unregister_node(node_id)?;
        self.arena.detach(node_id)?;
        self.arena.remove_subtree(node_id)?;
        self.history.push(Operation::Remove {
            node_id: name.to_string(),
        });
        Ok(())
    }

    pub fn set_attr(&mut self, name: &str, attr: &str, value: &str) -> Result<()> {
        let node_id = self.resolve_id(name)?;
        self.arena.get_mut(node_id)?.attrs.set_from_svg(attr, value);
        self.history.push(Operation::SetAttr {
            node_id: name.to_string(),
            attr: attr.to_string(),
            value: value.to_string(),
        });
        Ok(())
    }

    pub fn rename(&mut self, old_name: &str, new_name: &str) -> Result<()> {
        if old_name == "root" {
            return Err(StrokError::InvalidOperation(
                "cannot rename root node".to_string(),
            ));
        }
        id::validate_id(new_name, &self.id_index)?;
        let node_id = self.resolve_id(old_name)?;
        self.id_index.remove(old_name);
        self.arena.get_mut(node_id)?.id = new_name.to_string();
        self.id_index.insert(new_name.to_string(), node_id);
        self.history.push(Operation::Rename {
            old_id: old_name.to_string(),
            new_id: new_name.to_string(),
        });
        Ok(())
    }

    pub fn reorder(&mut self, name: &str, new_index: usize) -> Result<()> {
        let node_id = self.resolve_id(name)?;
        let parent_id = self
            .arena
            .get(node_id)?
            .parent
            .ok_or_else(|| StrokError::InvalidOperation("node has no parent".to_string()))?;

        self.arena
            .get_mut(parent_id)?
            .children
            .retain(|&id| id != node_id);
        let parent = self.arena.get_mut(parent_id)?;
        let idx = new_index.min(parent.children.len());
        parent.children.insert(idx, node_id);
        self.history.push(Operation::Reorder {
            node_id: name.to_string(),
            index: idx,
        });
        Ok(())
    }

    pub fn reparent(
        &mut self,
        name: &str,
        new_parent_name: &str,
        index: Option<usize>,
    ) -> Result<()> {
        let node_id = self.resolve_id(name)?;
        let new_parent_id = self.resolve_id(new_parent_name)?;
        self.arena.reparent(node_id, new_parent_id, index)?;
        self.history.push(Operation::Reparent {
            node_id: name.to_string(),
            new_parent: new_parent_name.to_string(),
            index,
        });
        Ok(())
    }

    pub fn group(&mut self, names: &[&str], group_id: Option<&str>) -> Result<String> {
        if names.is_empty() {
            return Err(StrokError::InvalidOperation(
                "must specify at least one element to group".to_string(),
            ));
        }

        let first_nid = self.resolve_id(names[0])?;
        let parent_id = self
            .arena
            .get(first_nid)?
            .parent
            .ok_or_else(|| StrokError::InvalidOperation("node has no parent".to_string()))?;

        for &name in &names[1..] {
            let nid = self.resolve_id(name)?;
            let p = self.arena.get(nid)?.parent;
            if p != Some(parent_id) {
                return Err(StrokError::InvalidOperation(
                    "all elements must share the same parent".to_string(),
                ));
            }
        }

        let first_index = self
            .arena
            .get(parent_id)?
            .children
            .iter()
            .position(|&id| id == first_nid)
            .ok_or_else(|| {
                StrokError::InvalidOperation(
                    "element not found among parent's children".to_string(),
                )
            })?;

        let mut group_node = SceneNode::new(group_id.unwrap_or("").to_string(), NodeKind::Group);
        if group_node.id.is_empty() {
            group_node.id = id::generate_id(&self.id_index);
        } else {
            id::validate_id(&group_node.id, &self.id_index)?;
        }
        let gid = self.arena.alloc(group_node);
        let gid_str = self.arena.get(gid)?.id.clone();
        self.id_index.insert(gid_str.clone(), gid);

        self.arena.insert_child(parent_id, first_index, gid)?;

        let node_ids: Vec<NodeId> = names
            .iter()
            .map(|n| self.resolve_id(n))
            .collect::<Result<Vec<_>>>()?;
        for nid in node_ids {
            self.arena.detach(nid)?;
            self.arena.append_child(gid, nid)?;
        }

        self.history.push(Operation::Group {
            children: names.iter().map(|s| s.to_string()).collect(),
            group_id: gid_str.clone(),
        });
        Ok(gid_str)
    }

    pub fn ungroup(&mut self, name: &str) -> Result<()> {
        let group_nid = self.resolve_id(name)?;
        if self.arena.get(group_nid)?.kind != NodeKind::Group {
            return Err(StrokError::InvalidOperation(format!(
                "cannot ungroup: element '{}' is not a group",
                name
            )));
        }

        let parent_id = self
            .arena
            .get(group_nid)?
            .parent
            .ok_or_else(|| StrokError::InvalidOperation("group has no parent".to_string()))?;

        let group_index = self
            .arena
            .get(parent_id)?
            .children
            .iter()
            .position(|&id| id == group_nid)
            .ok_or_else(|| {
                StrokError::InvalidOperation("group not found among parent's children".to_string())
            })?;

        let children: Vec<NodeId> = self.arena.get(group_nid)?.children.clone();

        self.arena
            .get_mut(parent_id)?
            .children
            .retain(|&id| id != group_nid);

        for (i, &child_id) in children.iter().enumerate() {
            self.arena.get_mut(child_id)?.parent = Some(parent_id);
            self.arena
                .get_mut(parent_id)?
                .children
                .insert(group_index + i, child_id);
        }

        self.id_index.remove(name);
        self.arena.get_mut(group_nid)?.children.clear();
        self.arena.remove(group_nid)?;

        self.history.push(Operation::Ungroup {
            group_id: name.to_string(),
        });
        Ok(())
    }

    fn rebuild_id_index(&mut self) {
        self.id_index.clear();
        self.register_node(self.root_id).ok();
    }

    // --- History inspection & replay (E3.3, `diff --since`) ---

    /// Number of operations recorded in the construction history (the op log).
    /// `Operation::New` counts as op 0, so a freshly-built document has length 1.
    pub fn history_len(&self) -> usize {
        self.history.ops().len()
    }

    /// Reconstruct the document state *after the first `n` operations* by
    /// replaying the op log against a fresh arena, sourcing the SVG content for
    /// content-bearing ops (append/prepend/insert/replace) from `self`'s current
    /// subtrees (node ids are stable, so this is exact). Used by `diff --since`
    /// to render a historical state and compare it against the current one.
    ///
    /// Only meaningful for arena (v2 / binary-format) documents whose op log
    /// persists. v3 DSL `.strok` files do not serialize the op log, so callers
    /// must handle the empty-history case (see `diff --since` in the CLI).
    pub fn replay_to(&self, n: usize) -> Result<Document> {
        let ops = self.history.ops();
        let n = n.min(ops.len());

        // Seed from the New op (op 0) if present, else current dimensions.
        let mut doc = match ops.first() {
            Some(Operation::New { width, height }) => Document::new(*width, *height),
            _ => Document::new(self.width, self.height),
        };
        // `Document::new` already pushed its own `New`; clear so we re-record.
        doc.history = OperationLog::new();
        doc.history.push(Operation::New {
            width: doc.width,
            height: doc.height,
        });

        for op in ops.iter().take(n).skip(1) {
            self.apply_replay_op(&mut doc, op)?;
            doc.history.push(op.clone());
        }
        Ok(doc)
    }

    /// Apply a single recorded op to a replay document, pulling node content
    /// from `self` where the op only recorded an id.
    fn apply_replay_op(&self, doc: &mut Document, op: &Operation) -> Result<()> {
        match op {
            Operation::New { .. } => {}
            Operation::Append { parent, node_id } => {
                let svg = self.subtree_svg(node_id)?;
                doc.append_svg(parent, &svg)?;
                doc.history.pop_last();
            }
            Operation::Prepend { parent, node_id } => {
                let svg = self.subtree_svg(node_id)?;
                doc.prepend_svg(parent, &svg)?;
                doc.history.pop_last();
            }
            Operation::Insert {
                parent,
                index,
                node_id,
            } => {
                let svg = self.subtree_svg(node_id)?;
                doc.insert_svg(parent, *index, &svg)?;
                doc.history.pop_last();
            }
            Operation::Remove { node_id } => {
                doc.remove(node_id)?;
                doc.history.pop_last();
            }
            Operation::SetAttr {
                node_id,
                attr,
                value,
            } => {
                doc.set_attr(node_id, attr, value)?;
                doc.history.pop_last();
            }
            Operation::Rename { old_id, new_id } => {
                doc.rename(old_id, new_id)?;
                doc.history.pop_last();
            }
            Operation::Replace { old_id, new_id } => {
                let svg = self.subtree_svg(new_id)?;
                doc.replace_svg(old_id, &svg)?;
                doc.history.pop_last();
            }
            Operation::Reorder { node_id, index } => {
                doc.reorder(node_id, *index)?;
                doc.history.pop_last();
            }
            Operation::Reparent {
                node_id,
                new_parent,
                index,
            } => {
                doc.reparent(node_id, new_parent, *index)?;
                doc.history.pop_last();
            }
            Operation::Group { children, group_id } => {
                let names: Vec<&str> = children.iter().map(|s| s.as_str()).collect();
                doc.group(&names, Some(group_id))?;
                doc.history.pop_last();
            }
            Operation::Ungroup { group_id } => {
                doc.ungroup(group_id)?;
                doc.history.pop_last();
            }
            // Geometry transforms recorded data-complete; arena docs don't carry
            // a placement model, so these are no-ops in replay (the v3 scene
            // path is where transforms live). Recording them keeps the log
            // faithful without altering arena geometry.
            Operation::Move { .. } | Operation::Rotate { .. } | Operation::Scale { .. } => {}
        }
        Ok(())
    }

    /// Emit a node's subtree as an SVG snippet (used to replay content ops).
    fn subtree_svg(&self, name: &str) -> Result<String> {
        let nid = self.resolve_id(name)?;
        Ok(crate::emit::emit_subtree(self, nid, None))
    }

    /// Save the document. Uses v3 DSL if a scene is present,
    /// otherwise falls back to the legacy binary format.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(ref scene) = self.scene {
            let text = crate::dsl_emit::emit_scene(scene);
            std::fs::write(path, text)?;
        } else {
            self.save_binary(path)?;
        }
        Ok(())
    }

    /// Save in the legacy binary format.
    #[allow(dead_code)]
    pub fn save_binary(&self, path: &Path) -> Result<()> {
        let file = StrokFile {
            magic: *MAGIC,
            version: VERSION,
            document: self.clone(),
        };
        let bytes =
            bincode::serialize(&file).map_err(|e| StrokError::Serialization(e.to_string()))?;
        std::fs::write(path, bytes)?;
        Ok(())
    }

    /// Load a .strok file. Detects v3 (starts with "documentsize") vs legacy formats.
    pub fn load(path: &Path) -> Result<Self> {
        let bytes = std::fs::read(path)?;
        if bytes.starts_with(MAGIC) {
            // Legacy binary format.
            let file: StrokFile = bincode::deserialize(&bytes)
                .map_err(|e| StrokError::Serialization(e.to_string()))?;
            if &file.magic != MAGIC {
                return Err(StrokError::Serialization("invalid file magic".to_string()));
            }
            let mut doc = file.document;
            doc.rebuild_id_index();
            Ok(doc)
        } else {
            let text = std::str::from_utf8(&bytes)
                .map_err(|e| StrokError::ParseError(format!("invalid UTF-8: {}", e)))?;
            Self::load_str_with_path(text, path)
        }
    }

    /// Load from a string. Detects v3 vs v2 format.
    pub fn load_str(text: &str) -> Result<Self> {
        // v3 format starts with "documentsize"
        let trimmed = text.trim_start();
        let first_word = trimmed.split_whitespace().next().unwrap_or("");

        if first_word == "documentsize" || first_word == "#" || first_word == "use" {
            let scene = crate::dsl_parse::parse_file(text)?;
            Ok(Document::from_scene(scene))
        } else {
            Err(StrokError::ParseError(
                "unrecognized file format (expected v3 DSL starting with 'documentsize')"
                    .to_string(),
            ))
        }
    }

    /// Load from a string with a file path, enabling `use` import resolution.
    pub fn load_str_with_path(text: &str, file_path: &Path) -> Result<Self> {
        let trimmed = text.trim_start();
        let first_word = trimmed.split_whitespace().next().unwrap_or("");

        if first_word == "documentsize" || first_word == "#" || first_word == "use" {
            let scene = crate::dsl_parse::parse_file_with_path(text, file_path)?;
            Ok(Document::from_scene(scene))
        } else {
            Err(StrokError::ParseError(
                "unrecognized file format (expected v3 DSL starting with 'documentsize')"
                    .to_string(),
            ))
        }
    }
}
