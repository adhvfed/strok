use crate::error::{Result, StrokError};
use crate::node::{NodeId, SceneNode};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Arena {
    nodes: Vec<Option<SceneNode>>,
    free: Vec<u32>,
}

impl Default for Arena {
    fn default() -> Self {
        Self::new()
    }
}

impl Arena {
    pub fn new() -> Self {
        Self {
            nodes: Vec::new(),
            free: Vec::new(),
        }
    }

    pub fn alloc(&mut self, node: SceneNode) -> NodeId {
        if let Some(idx) = self.free.pop() {
            self.nodes[idx as usize] = Some(node);
            NodeId(idx)
        } else {
            let idx = self.nodes.len() as u32;
            self.nodes.push(Some(node));
            NodeId(idx)
        }
    }

    pub fn get(&self, id: NodeId) -> Result<&SceneNode> {
        self.nodes
            .get(id.0 as usize)
            .and_then(|slot| slot.as_ref())
            .ok_or(StrokError::InvalidNodeIndex(id.0))
    }

    pub fn get_mut(&mut self, id: NodeId) -> Result<&mut SceneNode> {
        self.nodes
            .get_mut(id.0 as usize)
            .and_then(|slot| slot.as_mut())
            .ok_or(StrokError::InvalidNodeIndex(id.0))
    }

    pub fn remove(&mut self, id: NodeId) -> Result<SceneNode> {
        let node = self
            .nodes
            .get_mut(id.0 as usize)
            .and_then(|slot| slot.take())
            .ok_or(StrokError::InvalidNodeIndex(id.0))?;
        self.free.push(id.0);
        Ok(node)
    }

    /// Recursively remove a node and all its descendants. Returns all removed nodes.
    pub fn remove_subtree(&mut self, id: NodeId) -> Result<Vec<(NodeId, SceneNode)>> {
        let children: Vec<NodeId> = self.get(id)?.children.clone();
        let mut removed = Vec::new();
        for child_id in children {
            removed.extend(self.remove_subtree(child_id)?);
        }
        let node = self.remove(id)?;
        removed.push((id, node));
        Ok(removed)
    }

    /// Append child_id as last child of parent_id.
    pub fn append_child(&mut self, parent_id: NodeId, child_id: NodeId) -> Result<()> {
        self.get_mut(child_id)?.parent = Some(parent_id);
        self.get_mut(parent_id)?.children.push(child_id);
        Ok(())
    }

    /// Prepend child_id as first child of parent_id.
    pub fn prepend_child(&mut self, parent_id: NodeId, child_id: NodeId) -> Result<()> {
        self.get_mut(child_id)?.parent = Some(parent_id);
        self.get_mut(parent_id)?.children.insert(0, child_id);
        Ok(())
    }

    /// Insert child_id at a specific index in parent_id's children.
    pub fn insert_child(
        &mut self,
        parent_id: NodeId,
        index: usize,
        child_id: NodeId,
    ) -> Result<()> {
        self.get_mut(child_id)?.parent = Some(parent_id);
        let parent = self.get_mut(parent_id)?;
        let idx = index.min(parent.children.len());
        parent.children.insert(idx, child_id);
        Ok(())
    }

    /// Remove child_id from its parent's children list (does not deallocate).
    pub fn detach(&mut self, child_id: NodeId) -> Result<()> {
        if let Some(parent_id) = self.get(child_id)?.parent {
            self.get_mut(parent_id)?
                .children
                .retain(|&id| id != child_id);
        }
        self.get_mut(child_id)?.parent = None;
        Ok(())
    }

    /// Move a node to a new parent at a specific index.
    pub fn reparent(
        &mut self,
        node_id: NodeId,
        new_parent_id: NodeId,
        index: Option<usize>,
    ) -> Result<()> {
        self.detach(node_id)?;
        match index {
            Some(i) => self.insert_child(new_parent_id, i, node_id),
            None => self.append_child(new_parent_id, node_id),
        }
    }
}
