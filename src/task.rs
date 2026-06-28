use crate::io::{HandleId, IoHandle};
use crate::value::Value;
use crate::vm::CallFrame;
use std::collections::HashMap;

/// Unique identifier for a task in the VM.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TaskId(pub u64);

impl TaskId {
    /// The root task always has id 0.
    #[must_use]
    pub const fn root() -> Self {
        Self(0)
    }

    #[must_use]
    pub const fn is_root(self) -> bool {
        self.0 == 0
    }
}

impl core::fmt::Display for TaskId {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "Task({})", self.0)
    }
}

/// Execution status of a task.
#[derive(Debug, Clone, PartialEq)]
pub enum TaskStatus {
    /// Task is actively executing or ready to run.
    Running,
    /// Task has yielded at the given instruction pointer.
    Yielded(usize),
    /// Task has completed execution (reached Return with no caller).
    Completed,
}

impl TaskStatus {
    #[must_use]
    pub fn is_running(&self) -> bool {
        matches!(self, Self::Running)
    }

    #[must_use]
    pub fn is_completed(&self) -> bool {
        matches!(self, Self::Completed)
    }

    #[must_use]
    pub fn is_yielded(&self) -> bool {
        matches!(self, Self::Yielded(_))
    }
}

/// A task owns its own operand stack, call frames, and I/O handles.
///
/// Tasks form a strict tree: each task has at most one parent and zero or
/// more children. The root task (id 0) has no parent.
#[derive(Debug, Clone)]
pub struct Task {
    pub id: TaskId,
    pub parent: Option<TaskId>,
    pub children: Vec<TaskId>,
    pub status: TaskStatus,
    /// Per-task operand stack.
    pub stack: Vec<Value>,
    /// Per-task call frames.
    pub frames: Vec<CallFrame>,
    /// Per-task I/O handle registry.
    pub io_handles: HashMap<HandleId, IoHandle>,
}

impl Task {
    /// Create a new task with the given id and parent.
    #[must_use]
    pub fn new(id: TaskId, parent: Option<TaskId>) -> Self {
        Self {
            id,
            parent,
            children: Vec::new(),
            status: TaskStatus::Running,
            stack: Vec::new(),
            frames: Vec::new(),
            io_handles: HashMap::new(),
        }
    }

    /// Whether this task is the root of the task tree.
    #[must_use]
    pub fn is_root(&self) -> bool {
        self.parent.is_none()
    }

    /// Number of direct children.
    #[must_use]
    pub fn child_count(&self) -> usize {
        self.children.len()
    }
}

/// Provides tree-navigation operations over a task registry.
///
/// `TaskTree` is a lightweight view: it borrows the registry and provides
/// parent/child queries without owning the data.
#[derive(Debug)]
pub struct TaskTree<'a> {
    registry: &'a HashMap<TaskId, Task>,
}

impl<'a> TaskTree<'a> {
    /// Create a new tree view over the given registry.
    #[must_use]
    pub fn new(registry: &'a HashMap<TaskId, Task>) -> Self {
        Self { registry }
    }

    /// Get a reference to a task by id.
    #[must_use]
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        self.registry.get(&id)
    }

    /// Get the parent of a task, if any.
    #[must_use]
    pub fn find_parent(&self, id: TaskId) -> Option<&Task> {
        self.registry
            .get(&id)?
            .parent
            .and_then(|pid| self.registry.get(&pid))
    }

    /// Get all children of a task.
    #[must_use]
    pub fn get_children(&self, id: TaskId) -> Vec<&Task> {
        self.registry
            .get(&id)
            .map(|t| {
                t.children
                    .iter()
                    .filter_map(|cid| self.registry.get(cid))
                    .collect()
            })
            .unwrap_or_default()
    }

    /// Get the root task.
    #[must_use]
    pub fn root(&self) -> Option<&Task> {
        self.registry.get(&TaskId::root())
    }

    /// Count all tasks in the registry.
    #[must_use]
    pub fn len(&self) -> usize {
        self.registry.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.registry.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_registry() -> HashMap<TaskId, Task> {
        let mut registry = HashMap::new();
        let root = Task::new(TaskId::root(), None);
        registry.insert(TaskId::root(), root);
        registry
    }

    #[test]
    fn task_id_root_is_zero() {
        assert_eq!(TaskId::root(), TaskId(0));
        assert!(TaskId::root().is_root());
        assert!(!TaskId(1).is_root());
    }

    #[test]
    fn task_id_display() {
        assert_eq!(format!("{}", TaskId(0)), "Task(0)");
        assert_eq!(format!("{}", TaskId(42)), "Task(42)");
    }

    #[test]
    fn task_status_predicates() {
        assert!(TaskStatus::Running.is_running());
        assert!(!TaskStatus::Running.is_completed());
        assert!(!TaskStatus::Running.is_yielded());

        assert!(TaskStatus::Completed.is_completed());
        assert!(!TaskStatus::Completed.is_yielded());

        assert!(TaskStatus::Yielded(5).is_yielded());
    }

    #[test]
    fn new_task_has_correct_defaults() {
        let task = Task::new(TaskId(1), Some(TaskId::root()));
        assert_eq!(task.id, TaskId(1));
        assert_eq!(task.parent, Some(TaskId::root()));
        assert!(task.children.is_empty());
        assert_eq!(task.status, TaskStatus::Running);
        assert!(task.stack.is_empty());
        assert!(task.frames.is_empty());
        assert!(task.io_handles.is_empty());
    }

    #[test]
    fn root_task_has_no_parent() {
        let task = Task::new(TaskId::root(), None);
        assert!(task.is_root());
        assert_eq!(task.child_count(), 0);
    }

    #[test]
    fn task_child_count() {
        let mut task = Task::new(TaskId(0), None);
        assert_eq!(task.child_count(), 0);
        task.children.push(TaskId(1));
        task.children.push(TaskId(2));
        assert_eq!(task.child_count(), 2);
    }

    // -- TaskTree tests --

    #[test]
    fn tree_root() {
        let registry = make_registry();
        let tree = TaskTree::new(&registry);
        assert!(tree.root().is_some());
        assert_eq!(tree.root().unwrap().id, TaskId::root());
    }

    #[test]
    fn tree_empty() {
        let registry: HashMap<TaskId, Task> = HashMap::new();
        let tree = TaskTree::new(&registry);
        assert!(tree.is_empty());
        assert_eq!(tree.len(), 0);
        assert!(tree.root().is_none());
    }

    #[test]
    fn tree_parent_lookup() {
        let mut registry = make_registry();
        let child = Task::new(TaskId(1), Some(TaskId::root()));
        registry.insert(TaskId(1), child);

        let tree = TaskTree::new(&registry);
        let parent = tree.find_parent(TaskId(1));
        assert!(parent.is_some());
        assert_eq!(parent.unwrap().id, TaskId::root());
    }

    #[test]
    fn tree_parent_of_root_is_none() {
        let registry = make_registry();
        let tree = TaskTree::new(&registry);
        assert!(tree.find_parent(TaskId::root()).is_none());
    }

    #[test]
    fn tree_children_query() {
        let mut registry = make_registry();

        let mut root = registry.remove(&TaskId::root()).unwrap();
        root.children.push(TaskId(1));
        root.children.push(TaskId(2));
        registry.insert(TaskId::root(), root);
        registry.insert(TaskId(1), Task::new(TaskId(1), Some(TaskId::root())));
        registry.insert(TaskId(2), Task::new(TaskId(2), Some(TaskId::root())));

        let tree = TaskTree::new(&registry);
        let children = tree.get_children(TaskId::root());
        assert_eq!(children.len(), 2);
        assert_eq!(children[0].id, TaskId(1));
        assert_eq!(children[1].id, TaskId(2));
    }

    #[test]
    fn tree_children_of_leaf_is_empty() {
        let mut registry = make_registry();
        let leaf = Task::new(TaskId(1), Some(TaskId::root()));
        registry.insert(TaskId(1), leaf);

        let tree = TaskTree::new(&registry);
        assert!(tree.get_children(TaskId(1)).is_empty());
    }

    #[test]
    fn tree_get_nonexistent() {
        let registry = make_registry();
        let tree = TaskTree::new(&registry);
        assert!(tree.get(TaskId(99)).is_none());
    }

    #[test]
    fn tree_len() {
        let mut registry = make_registry();
        registry.insert(TaskId(1), Task::new(TaskId(1), Some(TaskId::root())));
        registry.insert(TaskId(2), Task::new(TaskId(2), Some(TaskId::root())));

        let tree = TaskTree::new(&registry);
        assert_eq!(tree.len(), 3);
    }
}
