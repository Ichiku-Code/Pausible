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
/// into this vector. The heap does not free individual objects;
/// that is the GC's job (Phase 2.2).
#[derive(Debug, Clone, Default)]
pub struct Heap {
    objects: Vec<HeapObject>,
}

impl Heap {
    #[must_use]
    pub fn new() -> Self {
        Self {
            objects: Vec::new(),
        }
    }

    /// Number of live objects (for GC heuristics).
    #[must_use]
    pub fn len(&self) -> usize {
        self.objects.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    /// Allocate a string on the heap, returning a typed handle.
    pub fn alloc_string(&mut self, data: String) -> Gc<StringObj> {
        let index = self.objects.len();
        self.objects
            .push(HeapObject::String(StringObj { data }));
        Gc::new(index)
    }

    /// Allocate a list on the heap, returning a typed handle.
    pub fn alloc_list(&mut self, elements: Vec<crate::value::Value>) -> Gc<ListObj> {
        let index = self.objects.len();
        self.objects
            .push(HeapObject::List(ListObj { elements }));
        Gc::new(index)
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
