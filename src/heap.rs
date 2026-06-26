use core::fmt;
use core::marker::PhantomData;

/// A typed handle to a heap-allocated object.
///
/// `Gc<T>` is a cheap, `Copy` index into the VM's heap. It does not
/// deallocate on drop; the GC handles that.
pub struct Gc<T> {
    pub(crate) index: usize,
    _marker: PhantomData<T>,
}

impl<T> Gc<T> {
    pub(crate) fn new(index: usize) -> Self {
        Self {
            index,
            _marker: PhantomData,
        }
    }
}

// Manual impls — PhantomData<T> prevents derive.
impl<T> Clone for Gc<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T> Copy for Gc<T> {}
impl<T> PartialEq for Gc<T> {
    fn eq(&self, other: &Self) -> bool {
        self.index == other.index
    }
}
impl<T> fmt::Debug for Gc<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Gc").field("index", &self.index).finish()
    }
}

// -- Heap object types --

/// A heap-allocated UTF-8 string.
#[derive(Debug, Clone)]
pub struct StringObj {
    pub data: String,
}

/// A heap-allocated dynamic list of Values.
#[derive(Debug, Clone)]
pub struct ListObj {
    pub elements: Vec<crate::value::Value>,
}

// -- HeapObject enum --

/// Discriminated union of all heap-allocatable object types.
#[derive(Debug, Clone)]
pub enum HeapObject {
    String(StringObj),
    List(ListObj),
}

// -- Heap --

/// Manages allocation and storage of all heap objects.
///
/// Objects are stored in a flat `Vec`. A `Gc<T>` is just an index
/// into this vector. Dead slots are tracked in `free_slots` for
/// reuse; the GC never deletes objects, preserving index stability.
///
/// Default GC threshold: 256 live objects.
const DEFAULT_GC_THRESHOLD: usize = 256;

#[derive(Debug, Clone)]
pub struct Heap {
    objects: Vec<HeapObject>,
    /// Per-object liveness tracking for the mark-sweep GC.
    marked: Vec<bool>,
    /// Indices of slots that have been freed and may be reused.
    free_slots: Vec<usize>,
    /// Trigger a GC cycle when the live object count exceeds this.
    gc_threshold: usize,
}

impl Default for Heap {
    fn default() -> Self {
        Self::new()
    }
}

impl Heap {
    #[must_use]
    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
            marked: Vec::new(),
            free_slots: Vec::new(),
            gc_threshold: DEFAULT_GC_THRESHOLD,
        }
    }

    /// Number of live objects (total minus free slots).
    #[must_use]
    pub fn len(&self) -> usize {
        self.objects.len().saturating_sub(self.free_slots.len())
    }

    /// Total capacity (including free slots).
    #[must_use]
    pub fn capacity(&self) -> usize {
        self.objects.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    /// Allocate a string on the heap, returning a typed handle.
    ///
    /// Reuses a free slot when available; otherwise appends to the
    /// object vector (index-stable).
    pub fn alloc_string(&mut self, data: String) -> Gc<StringObj> {
        let obj = HeapObject::String(StringObj { data });
        if let Some(idx) = self.free_slots.pop() {
            self.objects[idx] = obj;
            self.marked[idx] = false;
            Gc::new(idx)
        } else {
            let idx = self.objects.len();
            self.objects.push(obj);
            self.marked.push(false);
            Gc::new(idx)
        }
    }

    /// Allocate a list on the heap, returning a typed handle.
    ///
    /// Reuses a free slot when available; otherwise appends to the
    /// object vector (index-stable).
    pub fn alloc_list(&mut self, elements: Vec<crate::value::Value>) -> Gc<ListObj> {
        let obj = HeapObject::List(ListObj { elements });
        if let Some(idx) = self.free_slots.pop() {
            self.objects[idx] = obj;
            self.marked[idx] = false;
            Gc::new(idx)
        } else {
            let idx = self.objects.len();
            self.objects.push(obj);
            self.marked.push(false);
            Gc::new(idx)
        }
    }

    /// Borrow a heap object by typed handle.
    #[must_use]
    pub fn get<T: HeapAccess>(&self, gc: Gc<T>) -> Option<&T> {
        self.objects.get(gc.index).and_then(|obj| T::from_heap(obj))
    }

    /// Mutably borrow a heap object by typed handle.
    pub fn get_mut<T: HeapAccess>(&mut self, gc: Gc<T>) -> Option<&mut T> {
        self.objects
            .get_mut(gc.index)
            .and_then(|obj| T::from_heap_mut(obj))
    }

    // -- GC (mark-sweep with free-list) --

    /// True when the live object count exceeds the GC threshold.
    #[must_use]
    pub fn should_gc(&self) -> bool {
        self.len() > self.gc_threshold
    }

    /// Mark the object at `idx` as reachable, then recursively mark
    /// its children (List elements that are heap references).
    pub(crate) fn mark(&mut self, idx: usize) {
        // Already marked, or out of bounds (should never happen for valid Gc).
        if self.marked.get(idx) != Some(&false) {
            return;
        }
        self.marked[idx] = true;

        // Collect child heap indices to avoid borrow-conflict with recursion.
        let children: Vec<usize> = match &self.objects[idx] {
            HeapObject::List(list) => list
                .elements
                .iter()
                .filter_map(|val| match val {
                    crate::value::Value::String(gc) => Some(gc.index),
                    crate::value::Value::List(gc) => Some(gc.index),
                    _ => None,
                })
                .collect(),
            HeapObject::String(_) => Vec::new(),
        };

        for child_idx in children {
            self.mark(child_idx);
        }
    }

    /// External root-scanning entry point: mark the heap object
    /// referenced by a `Value` if that value is a heap type.
    pub(crate) fn mark_value(&mut self, val: &crate::value::Value) {
        match val {
            crate::value::Value::String(gc) => self.mark(gc.index),
            crate::value::Value::List(gc) => self.mark(gc.index),
            _ => {}
        }
    }

    /// Sweep: rebuild the free-slot list from unmarked objects
    /// and reset marks on survivors for the next GC cycle.
    fn sweep(&mut self) {
        self.free_slots.clear();
        for (i, m) in self.marked.iter_mut().enumerate() {
            if *m {
                *m = false; // survivor: reset mark
            } else {
                self.free_slots.push(i); // dead: available for reuse
            }
        }
    }

    /// Run a full mark-sweep GC cycle.
    ///
    /// 1. Reset all marks.
    /// 2. Mark roots (caller must invoke `mark_value` on every root).
    /// 3. Sweep — unmarked slots go into the free list.
    ///
    /// Returns whether the object at `idx` is currently marked.
    pub(crate) fn is_marked(&self, idx: usize) -> bool {
        self.marked.get(idx).copied().unwrap_or(false)
    }

    /// Returns a reference to the object at `idx`.
    pub(crate) fn get_object(&self, idx: usize) -> Option<&HeapObject> {
        self.objects.get(idx)
    }

    pub fn collect_garbage_after_mark(&mut self) {
        self.sweep();
    }

    /// Reset all marks before a new mark phase begins.
    pub(crate) fn reset_marks(&mut self) {
        for m in &mut self.marked {
            *m = false;
        }
    }
}

// -- HeapAccess trait: enables typed projection from HeapObject --

/// Implemented by types that can be projected from a `HeapObject`.
pub trait HeapAccess: Sized {
    fn from_heap(obj: &HeapObject) -> Option<&Self>;
    fn from_heap_mut(obj: &mut HeapObject) -> Option<&mut Self>;
}

impl HeapAccess for StringObj {
    fn from_heap(obj: &HeapObject) -> Option<&Self> {
        match obj {
            HeapObject::String(s) => Some(s),
            HeapObject::List(_) => None,
        }
    }
    fn from_heap_mut(obj: &mut HeapObject) -> Option<&mut Self> {
        match obj {
            HeapObject::String(s) => Some(s),
            HeapObject::List(_) => None,
        }
    }
}

impl HeapAccess for ListObj {
    fn from_heap(obj: &HeapObject) -> Option<&Self> {
        match obj {
            HeapObject::List(l) => Some(l),
            HeapObject::String(_) => None,
        }
    }
    fn from_heap_mut(obj: &mut HeapObject) -> Option<&mut Self> {
        match obj {
            HeapObject::List(l) => Some(l),
            HeapObject::String(_) => None,
        }
    }
}
