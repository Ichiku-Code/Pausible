// Value serialization uses position→Gc and Gc→position maps that grow
// with the heap; u32 is generous enough for all practical heap sizes.
#![allow(clippy::cast_possible_truncation)]

use std::collections::HashMap;

use crate::heap::Heap;
use crate::value::Value;

// -- wire format tags --

const TAG_INT: u8 = 0x00;
const TAG_FLOAT: u8 = 0x01;
const TAG_BOOL: u8 = 0x02;
const TAG_NULL: u8 = 0x03;
const TAG_STRING: u8 = 0x04;
const TAG_LIST: u8 = 0x05;

// -- context types --

/// Serialization context: accumulates output buffer and tracks
/// which heap objects have been encountered.
///
/// The `object_map` records each Gc index the first time it appears,
/// assigning a monotonically increasing sequence number. This is the
/// foundation for the snapshot heap section (Phase 2.4): the map tells
/// the snapshot layer which objects to emit and in what order.
#[derive(Debug, Clone)]
pub struct SerCtx<'a> {
    /// Accumulated serialized bytes.
    pub buf: Vec<u8>,
    /// Maps original Gc index → encounter order (0, 1, 2, …).
    /// Populated lazily as heap-typed Values are first seen.
    pub object_map: HashMap<usize, u32>,
    /// Borrowed reference to the VM's heap for reading object data.
    pub heap: &'a Heap,
    next_seq: u32,
}

impl<'a> SerCtx<'a> {
    #[must_use]
    pub fn new(heap: &'a Heap) -> Self {
        Self {
            buf: Vec::new(),
            object_map: HashMap::new(),
            heap,
            next_seq: 0,
        }
    }

    /// Record a Gc index in the `object_map`, returning the assigned
    /// sequence number (new or existing).
    fn record_object(&mut self, gc_index: usize) -> u32 {
        let next = &mut self.next_seq;
        *self
            .object_map
            .entry(gc_index)
            .or_insert_with(|| {
                let seq = *next;
                *next += 1;
                seq
            })
    }
}

/// Deserialization context: reads from input and owns a `Heap` for
/// reconstructing heap-allocated objects.
///
/// The `object_map` is the deserialization counterpart of [`SerCtx`]:
/// when the snapshot heap section is rebuilt first (Phase 2.4), this
/// map translates sequence numbers back to new Gc handles.
#[derive(Debug, Clone)]
pub struct DeCtx<'a> {
    data: &'a [u8],
    pos: usize,
    /// Heap used to allocate String / List objects during deserialization.
    pub heap: Heap,
    /// Maps sequence number → newly allocated Gc index.
    pub object_map: HashMap<u32, usize>,
}

impl<'a> DeCtx<'a> {
    #[must_use]
    pub fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            heap: Heap::new(),
            object_map: HashMap::new(),
        }
    }

    /// Record a (sequence, Gc-index) pair after allocating a heap object.
    fn record_object(&mut self, seq: u32, gc_index: usize) {
        self.object_map.insert(seq, gc_index);
    }
}

// -- errors --

/// Errors that can occur during deserialization.
#[derive(Debug, Clone, PartialEq)]
pub enum SerError {
    /// Reached end of data while expecting more bytes.
    UnexpectedEof,
    /// Unknown type tag byte encountered.
    UnknownValueTag(u8),
}

impl core::fmt::Display for SerError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::UnexpectedEof => write!(f, "unexpected end of serialized data"),
            Self::UnknownValueTag(b) => write!(f, "unknown value tag: {b:#04X}"),
        }
    }
}

impl core::error::Error for SerError {}

// -- Serializable trait --

/// Types that can be serialized and deserialized through
/// [`SerCtx`] / [`DeCtx`].
pub trait Serializable: Sized {
    /// Write `self` into `ctx.buf`.
    fn serialize(&self, ctx: &mut SerCtx<'_>);

    /// Read and reconstruct `Self` from `ctx`.
    ///
    /// # Errors
    ///
    /// Returns `SerError` if the data is malformed or truncated.
    fn deserialize(ctx: &mut DeCtx<'_>) -> Result<Self, SerError>;
}

// -- low-level read/write helpers --

impl SerCtx<'_> {
    fn write_u8(&mut self, b: u8) {
        self.buf.push(b);
    }

    fn write_u32(&mut self, v: u32) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_i64(&mut self, v: i64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }

    fn write_f64(&mut self, v: f64) {
        self.buf.extend_from_slice(&v.to_le_bytes());
    }
}

impl DeCtx<'_> {
    fn read_u8(&mut self) -> Result<u8, SerError> {
        let b = self.data.get(self.pos).ok_or(SerError::UnexpectedEof)?;
        self.pos += 1;
        Ok(*b)
    }

    fn read_u32(&mut self) -> Result<u32, SerError> {
        let end = self.pos.checked_add(4).ok_or(SerError::UnexpectedEof)?;
        let bytes: [u8; 4] = self
            .data
            .get(self.pos..end)
            .ok_or(SerError::UnexpectedEof)?
            .try_into()
            .unwrap();
        self.pos = end;
        Ok(u32::from_le_bytes(bytes))
    }

    fn read_i64(&mut self) -> Result<i64, SerError> {
        let end = self.pos.checked_add(8).ok_or(SerError::UnexpectedEof)?;
        let bytes: [u8; 8] = self
            .data
            .get(self.pos..end)
            .ok_or(SerError::UnexpectedEof)?
            .try_into()
            .unwrap();
        self.pos = end;
        Ok(i64::from_le_bytes(bytes))
    }

    fn read_f64(&mut self) -> Result<f64, SerError> {
        let end = self.pos.checked_add(8).ok_or(SerError::UnexpectedEof)?;
        let bytes: [u8; 8] = self
            .data
            .get(self.pos..end)
            .ok_or(SerError::UnexpectedEof)?
            .try_into()
            .unwrap();
        self.pos = end;
        Ok(f64::from_le_bytes(bytes))
    }
}

// -- Value implementation --

/// Heap objects are serialized inline: String writes its UTF-8 data
/// directly, List writes its elements recursively.  The `object_map`
/// is populated alongside so that the snapshot layer (Phase 2.4) can
/// later extract a deduplicated heap section from the stream.
impl Serializable for Value {
    fn serialize(&self, ctx: &mut SerCtx<'_>) {
        match self {
            Self::Int(v) => {
                ctx.write_u8(TAG_INT);
                ctx.write_i64(*v);
            }
            Self::Float(v) => {
                ctx.write_u8(TAG_FLOAT);
                ctx.write_f64(*v);
            }
            Self::Bool(v) => {
                ctx.write_u8(TAG_BOOL);
                ctx.write_u8(u8::from(*v));
            }
            Self::Null => {
                ctx.write_u8(TAG_NULL);
            }
            Self::String(gc) => {
                ctx.write_u8(TAG_STRING);
                // Track object identity for snapshot use.
                let _seq = ctx.record_object(gc.index);
                let s = ctx.heap.get(*gc).expect("valid Gc handle");
                ctx.write_u32(s.data.len() as u32);
                ctx.buf.extend_from_slice(s.data.as_bytes());
            }
            Self::List(gc) => {
                ctx.write_u8(TAG_LIST);
                let _seq = ctx.record_object(gc.index);
                let list = ctx.heap.get(*gc).expect("valid Gc handle");
                ctx.write_u32(list.elements.len() as u32);
                for elem in &list.elements {
                    elem.serialize(ctx);
                }
            }
        }
    }

    fn deserialize(ctx: &mut DeCtx<'_>) -> Result<Self, SerError> {
        let tag = ctx.read_u8()?;
        match tag {
            TAG_INT => {
                let v = ctx.read_i64()?;
                Ok(Value::Int(v))
            }
            TAG_FLOAT => {
                let v = ctx.read_f64()?;
                Ok(Value::Float(v))
            }
            TAG_BOOL => {
                let b = ctx.read_u8()?;
                Ok(Value::Bool(b != 0))
            }
            TAG_NULL => Ok(Value::Null),
            TAG_STRING => {
                let len = ctx.read_u32()? as usize;
                let end = ctx.pos.checked_add(len).ok_or(SerError::UnexpectedEof)?;
                let bytes = ctx.data.get(ctx.pos..end).ok_or(SerError::UnexpectedEof)?;
                ctx.pos = end;
                let s = String::from_utf8_lossy(bytes).into_owned();
                let gc = ctx.heap.alloc_string(s);
                // Sequence number for object tracking: we use the next
                // map length as the sequence (monotonically increasing).
                let seq = ctx.object_map.len() as u32;
                ctx.record_object(seq, gc.index);
                Ok(Value::String(gc))
            }
            TAG_LIST => {
                let count = ctx.read_u32()? as usize;
                let mut elements = Vec::with_capacity(count);
                for _ in 0..count {
                    elements.push(Value::deserialize(ctx)?);
                }
                let gc = ctx.heap.alloc_list(elements);
                let seq = ctx.object_map.len() as u32;
                ctx.record_object(seq, gc.index);
                Ok(Value::List(gc))
            }
            _ => Err(SerError::UnknownValueTag(tag)),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::heap::Heap;

    // -- helpers --

    fn roundtrip(val: &Value, heap: &Heap) -> Value {
        let mut ser_ctx = SerCtx::new(heap);
        val.serialize(&mut ser_ctx);
        let mut de_ctx = DeCtx::new(&ser_ctx.buf);
        Value::deserialize(&mut de_ctx).unwrap()
    }

    // -- primitive roundtrips --

    #[test]
    fn roundtrip_int() {
        let heap = Heap::new();
        let v = Value::Int(42);
        assert_eq!(roundtrip(&v, &heap), Value::Int(42));
    }

    #[test]
    fn roundtrip_int_negative() {
        let heap = Heap::new();
        let v = Value::Int(-999);
        assert_eq!(roundtrip(&v, &heap), Value::Int(-999));
    }

    #[test]
    fn roundtrip_float() {
        let heap = Heap::new();
        let v = Value::Float(1.5);
        let restored = roundtrip(&v, &heap);
        match restored {
            Value::Float(f) => assert!((f - 1.5).abs() < f64::EPSILON),
            other => panic!("expected Float, got {other:?}"),
        }
    }

    #[test]
    fn roundtrip_bool_true() {
        let heap = Heap::new();
        let v = Value::Bool(true);
        assert_eq!(roundtrip(&v, &heap), Value::Bool(true));
    }

    #[test]
    fn roundtrip_bool_false() {
        let heap = Heap::new();
        let v = Value::Bool(false);
        assert_eq!(roundtrip(&v, &heap), Value::Bool(false));
    }

    #[test]
    fn roundtrip_null() {
        let heap = Heap::new();
        assert_eq!(roundtrip(&Value::Null, &heap), Value::Null);
    }

    // -- heap type roundtrips (inline, content-based comparison) --

    #[test]
    fn roundtrip_string() {
        let mut heap = Heap::new();
        let gc = heap.alloc_string("hello world".into());
        let v = Value::String(gc);

        let mut ser_ctx = SerCtx::new(&heap);
        v.serialize(&mut ser_ctx);
        let mut de_ctx = DeCtx::new(&ser_ctx.buf);
        let restored = Value::deserialize(&mut de_ctx).unwrap();

        assert!(restored.is_string());
        if let Value::String(gc2) = &restored {
            let s = de_ctx.heap.get(*gc2).unwrap();
            assert_eq!(s.data, "hello world");
        } else {
            panic!("expected String");
        }
    }

    #[test]
    fn roundtrip_empty_string() {
        let mut heap = Heap::new();
        let gc = heap.alloc_string(String::new());
        let v = Value::String(gc);

        let mut ser_ctx = SerCtx::new(&heap);
        v.serialize(&mut ser_ctx);
        let mut de_ctx = DeCtx::new(&ser_ctx.buf);
        let restored = Value::deserialize(&mut de_ctx).unwrap();

        if let Value::String(gc2) = &restored {
            assert_eq!(de_ctx.heap.get(*gc2).unwrap().data, "");
        } else {
            panic!("expected String");
        }
    }

    #[test]
    fn roundtrip_string_unicode() {
        let mut heap = Heap::new();
        let gc = heap.alloc_string("你好世界 🌍".into());
        let v = Value::String(gc);

        let mut ser_ctx = SerCtx::new(&heap);
        v.serialize(&mut ser_ctx);
        let mut de_ctx = DeCtx::new(&ser_ctx.buf);
        let restored = Value::deserialize(&mut de_ctx).unwrap();

        if let Value::String(gc2) = &restored {
            assert_eq!(de_ctx.heap.get(*gc2).unwrap().data, "你好世界 🌍");
        } else {
            panic!("expected String");
        }
    }

    #[test]
    fn roundtrip_list_of_ints() {
        let mut heap = Heap::new();
        let gc = heap.alloc_list(vec![Value::Int(1), Value::Int(2), Value::Int(3)]);
        let v = Value::List(gc);

        let mut ser_ctx = SerCtx::new(&heap);
        v.serialize(&mut ser_ctx);
        let mut de_ctx = DeCtx::new(&ser_ctx.buf);
        let restored = Value::deserialize(&mut de_ctx).unwrap();

        assert!(restored.is_list());
        if let Value::List(gc2) = &restored {
            let list = de_ctx.heap.get(*gc2).unwrap();
            assert_eq!(
                list.elements,
                vec![Value::Int(1), Value::Int(2), Value::Int(3)]
            );
        } else {
            panic!("expected List");
        }
    }

    #[test]
    fn roundtrip_mixed_list() {
        let mut heap = Heap::new();
        let inner = heap.alloc_string("nested".into());
        let gc = heap.alloc_list(vec![
            Value::Int(42),
            Value::Bool(true),
            Value::Null,
            Value::String(inner),
        ]);
        let v = Value::List(gc);

        let mut ser_ctx = SerCtx::new(&heap);
        v.serialize(&mut ser_ctx);
        let mut de_ctx = DeCtx::new(&ser_ctx.buf);
        let restored = Value::deserialize(&mut de_ctx).unwrap();

        if let Value::List(gc2) = &restored {
            let list = de_ctx.heap.get(*gc2).unwrap();
            assert_eq!(list.elements.len(), 4);
            assert_eq!(list.elements[0], Value::Int(42));
            assert_eq!(list.elements[1], Value::Bool(true));
            assert_eq!(list.elements[2], Value::Null);
            assert!(list.elements[3].is_string());
            if let Value::String(inner_gc) = &list.elements[3] {
                assert_eq!(de_ctx.heap.get(*inner_gc).unwrap().data, "nested");
            } else {
                panic!("expected String at index 3");
            }
        } else {
            panic!("expected List");
        }
    }

    #[test]
    fn roundtrip_nested_list() {
        let mut heap = Heap::new();
        let inner = heap.alloc_list(vec![Value::Int(1), Value::Int(2)]);
        let outer = heap.alloc_list(vec![Value::List(inner), Value::Int(3)]);
        let v = Value::List(outer);

        let mut ser_ctx = SerCtx::new(&heap);
        v.serialize(&mut ser_ctx);
        let mut de_ctx = DeCtx::new(&ser_ctx.buf);
        let restored = Value::deserialize(&mut de_ctx).unwrap();

        if let Value::List(outer_gc) = &restored {
            let outer_list = de_ctx.heap.get(*outer_gc).unwrap();
            assert_eq!(outer_list.elements.len(), 2);
            if let Value::List(inner_gc) = &outer_list.elements[0] {
                let inner_list = de_ctx.heap.get(*inner_gc).unwrap();
                assert_eq!(inner_list.elements, vec![Value::Int(1), Value::Int(2)]);
            } else {
                panic!("expected nested List");
            }
            assert_eq!(outer_list.elements[1], Value::Int(3));
        } else {
            panic!("expected List");
        }
    }

    // -- object_map tracking --

    #[test]
    fn object_map_tracks_unique_heap_objects() {
        let mut heap = Heap::new();
        let gc = heap.alloc_string("shared".into());
        let v = Value::String(gc);

        let mut ser_ctx = SerCtx::new(&heap);
        // Serialize the same Gc twice — the map should record it once.
        v.serialize(&mut ser_ctx);
        v.serialize(&mut ser_ctx);

        // Only one unique string object, so map size = 1.
        assert_eq!(ser_ctx.object_map.len(), 1);
        assert_eq!(ser_ctx.object_map.get(&gc.index), Some(&0));
    }

    #[test]
    fn object_map_distinguishes_different_objects() {
        let mut heap = Heap::new();
        let a = heap.alloc_string("alpha".into());
        let b = heap.alloc_string("beta".into());

        let mut ser_ctx = SerCtx::new(&heap);
        Value::String(a).serialize(&mut ser_ctx);
        Value::String(b).serialize(&mut ser_ctx);

        assert_eq!(ser_ctx.object_map.len(), 2);
        assert_ne!(
            ser_ctx.object_map.get(&a.index),
            ser_ctx.object_map.get(&b.index)
        );
    }

    // -- error cases --

    #[test]
    fn deserialize_empty_data_fails() {
        let mut ctx = DeCtx::new(&[]);
        let err = Value::deserialize(&mut ctx).unwrap_err();
        assert!(matches!(err, SerError::UnexpectedEof));
    }

    #[test]
    fn deserialize_unknown_tag_fails() {
        let mut ctx = DeCtx::new(&[0xFF]);
        let err = Value::deserialize(&mut ctx).unwrap_err();
        assert!(matches!(err, SerError::UnknownValueTag(0xFF)));
    }

    #[test]
    fn deserialize_truncated_int_fails() {
        // TAG_INT followed by only 4 bytes (needs 8).
        let data = [TAG_INT, 0x01, 0x00, 0x00, 0x00];
        let mut ctx = DeCtx::new(&data);
        let err = Value::deserialize(&mut ctx).unwrap_err();
        assert!(matches!(err, SerError::UnexpectedEof));
    }

    #[test]
    fn deserialize_truncated_string_fails() {
        // TAG_STRING, len=10, but only 3 bytes follow.
        let mut data = vec![TAG_STRING];
        data.extend_from_slice(&10u32.to_le_bytes());
        data.extend_from_slice(b"abc");
        let mut ctx = DeCtx::new(&data);
        let err = Value::deserialize(&mut ctx).unwrap_err();
        assert!(matches!(err, SerError::UnexpectedEof));
    }

    #[test]
    fn ser_error_display() {
        assert_eq!(
            format!("{}", SerError::UnexpectedEof),
            "unexpected end of serialized data"
        );
        assert_eq!(
            format!("{}", SerError::UnknownValueTag(0xFF)),
            "unknown value tag: 0xFF"
        );
    }

    #[test]
    fn roundtrip_empty_list() {
        let mut heap = Heap::new();
        let gc = heap.alloc_list(vec![]);
        let v = Value::List(gc);

        let mut ser_ctx = SerCtx::new(&heap);
        v.serialize(&mut ser_ctx);
        let mut de_ctx = DeCtx::new(&ser_ctx.buf);
        let restored = Value::deserialize(&mut de_ctx).unwrap();

        if let Value::List(gc2) = &restored {
            assert!(de_ctx.heap.get(*gc2).unwrap().elements.is_empty());
        } else {
            panic!("expected List");
        }
    }

    #[test]
    fn roundtrip_int_max_min() {
        let heap = Heap::new();
        for val in [i64::MIN, 0, i64::MAX] {
            assert_eq!(roundtrip(&Value::Int(val), &heap), Value::Int(val));
        }
    }
}
