use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Operation {
    New {
        width: f64,
        height: f64,
    },
    Append {
        parent: String,
        node_id: String,
    },
    Prepend {
        parent: String,
        node_id: String,
    },
    Insert {
        parent: String,
        index: usize,
        node_id: String,
    },
    Remove {
        node_id: String,
    },
    SetAttr {
        node_id: String,
        attr: String,
        value: String,
    },
    Rename {
        old_id: String,
        new_id: String,
    },
    Replace {
        old_id: String,
        new_id: String,
    },
    Reorder {
        node_id: String,
        index: usize,
    },
    Reparent {
        node_id: String,
        new_parent: String,
        index: Option<usize>,
    },
    Group {
        children: Vec<String>,
        group_id: String,
    },
    Ungroup {
        group_id: String,
    },
    Move {
        node_id: String,
        dx: f64,
        dy: f64,
    },
    Rotate {
        node_id: String,
        degrees: f64,
    },
    Scale {
        node_id: String,
        factor: f64,
    },
}

impl fmt::Display for Operation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Operation::New { width, height } => write!(f, "new ({width}x{height})"),
            Operation::Append { parent, node_id } => write!(f, "append {parent} {node_id}"),
            Operation::Prepend { parent, node_id } => write!(f, "prepend {parent} {node_id}"),
            Operation::Insert {
                parent,
                index,
                node_id,
            } => write!(f, "insert {parent} {index} {node_id}"),
            Operation::Remove { node_id } => write!(f, "remove {node_id}"),
            Operation::SetAttr {
                node_id,
                attr,
                value,
            } => write!(f, "set {node_id} --{attr} \"{value}\""),
            Operation::Rename { old_id, new_id } => write!(f, "rename {old_id} {new_id}"),
            Operation::Replace { old_id, new_id } => write!(f, "replace {old_id} → {new_id}"),
            Operation::Reorder { node_id, index } => write!(f, "reorder {node_id} {index}"),
            Operation::Reparent {
                node_id,
                new_parent,
                ..
            } => write!(f, "reparent {node_id} → {new_parent}"),
            Operation::Group { group_id, .. } => write!(f, "group → {group_id}"),
            Operation::Ungroup { group_id } => write!(f, "ungroup {group_id}"),
            Operation::Move { node_id, dx, dy } => write!(f, "move {node_id} {dx} {dy}"),
            Operation::Rotate { node_id, degrees } => write!(f, "rotate {node_id} {degrees}"),
            Operation::Scale { node_id, factor } => write!(f, "scale {node_id} {factor}"),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OperationLog {
    ops: Vec<Operation>,
    cursor: usize,
}

impl Default for OperationLog {
    fn default() -> Self {
        Self::new()
    }
}

impl OperationLog {
    pub fn new() -> Self {
        Self {
            ops: Vec::new(),
            cursor: 0,
        }
    }

    pub fn push(&mut self, op: Operation) {
        // Truncate any redo history
        self.ops.truncate(self.cursor);
        self.ops.push(op);
        self.cursor = self.ops.len();
    }

    pub fn ops(&self) -> &[Operation] {
        &self.ops
    }

    /// Remove the most recently pushed op (history replay helper, E3.3). Used
    /// when a mutator records its own op but the caller wants to substitute the
    /// canonical recorded op instead.
    pub fn pop_last(&mut self) -> Option<Operation> {
        let op = self.ops.pop();
        self.cursor = self.ops.len();
        op
    }

    pub fn cursor(&self) -> usize {
        self.cursor
    }

    pub fn can_undo(&self) -> bool {
        self.cursor > 0
    }

    pub fn can_redo(&self) -> bool {
        self.cursor < self.ops.len()
    }

    pub fn undo_op(&mut self) -> Option<&Operation> {
        if self.can_undo() {
            self.cursor -= 1;
            Some(&self.ops[self.cursor])
        } else {
            None
        }
    }

    pub fn redo_op(&mut self) -> Option<&Operation> {
        if self.can_redo() {
            let op = &self.ops[self.cursor];
            self.cursor += 1;
            Some(op)
        } else {
            None
        }
    }
}
