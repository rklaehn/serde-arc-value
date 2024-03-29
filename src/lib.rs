#[macro_use]
extern crate serde;
extern crate ordered_float;

#[cfg(test)]
#[macro_use]
extern crate serde_derive;

use ordered_float::OrderedFloat;
use serde::Deserialize;
use std::cmp::Ordering;
use std::collections::{BTreeMap, HashSet};
use std::fmt::Display;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

pub use de::*;
pub use ser::*;

mod de;
mod ser;

#[derive(Clone, Debug)]
pub enum Value {
    Unit,

    Bool(bool),

    U8(u8),
    U16(u16),
    U32(u32),
    U64(u64),

    I8(i8),
    I16(i16),
    I32(i32),
    I64(i64),

    F32(f32),
    F64(f64),

    Char(char),

    Option(Option<Box<Value>>),
    Newtype(Box<Value>),

    // complex, possibly shared, values
    String(Arc<String>),
    Bytes(Arc<Vec<u8>>),
    Seq(Arc<Vec<Value>>),
    Map(Arc<KV>),
}

pub trait Deduplicator {
    fn dedup(&mut self, value: Value) -> Value;
}

#[derive(Clone, Debug)]
pub struct Dedup {
    blobs: HashSet<Arc<Vec<u8>>>,
    strings: HashSet<Arc<String>>,
    vectors: HashSet<Arc<Vec<Value>>>,
    objects: HashSet<Arc<KV>>,
}

impl Dedup {
    pub fn new() -> Dedup {
        Dedup {
            blobs: HashSet::new(),
            strings: HashSet::new(),
            vectors: HashSet::new(),
            objects: HashSet::new(),
        }
    }

    fn strings(&self) -> Vec<(String, usize)> {
        self.strings.iter().cloned().map(|x| (x.as_ref().clone(), Arc::strong_count(&x))).collect()
    }

    fn objects(&self) -> Vec<(KV, usize)> {
        self.objects.iter().cloned().map(|x| (x.as_ref().clone(), Arc::strong_count(&x))).collect()
    }

    fn vectors(&self) -> Vec<(Vec<Value>, usize)> {
        self.vectors.iter().cloned().map(|x| (x.as_ref().clone(), Arc::strong_count(&x))).collect()
    }

    fn size(&self) -> usize {
        let mut res: usize = 0;
        for blob in self.blobs.iter() {
            res += blob.len();
        }
        for string in self.strings.iter() {
            res += string.len();
        }
        for vector in self.vectors.iter() {
            res += vector.len() * std::mem::size_of::<Value>();
        }
        for object in self.objects.iter() {
            res += std::mem::size_of::<KV>();
            let KV(_,v) = object.as_ref();
            res += v.len() * std::mem::size_of::<Value>();
        }
        res
    }

    fn dedup_value_vec(&mut self, vec: Vec<Value>) -> Vec<Value> {
        vec.into_iter().map(|x| self.dedup(x)).collect()
    }

    fn dedup_blob(&mut self, value: Arc<Vec<u8>>) -> Arc<Vec<u8>> {
        match self.blobs.get(value.as_ref()) {
            Some(value) => value.clone(),
            None => {
                self.blobs.insert(value.clone());
                value
            }
        }
    }

    fn dedup_string(&mut self, value: Arc<String>) -> Arc<String> {
        match self.strings.get(value.as_ref()) {
            Some(value) => value.clone(),
            None => {
                self.strings.insert(value.clone());
                value
            }
        }
    }

    fn dedup_seq(&mut self, value: Arc<Vec<Value>>) -> Arc<Vec<Value>> {
        match self.vectors.get(value.as_ref()) {
            Some(value) => value.clone(),
            None => {
                self.vectors.insert(value.clone());
                value
            }
        }
    }

    fn dedup_map(&mut self, value: Arc<KV>) -> Arc<KV> {
        match self.objects.get(value.as_ref()) {
            Some(value) => value.clone(),
            None => {
                self.objects.insert(value.clone());
                value
            }
        }
    }
}

impl Deduplicator for Dedup {
    fn dedup(&mut self, value: Value) -> Value {
        match value {
            Value::Bytes(v) => Value::Bytes(self.dedup_blob(v)),
            Value::String(v) => Value::String(self.dedup_string(v)),
            Value::Seq(elements) => {
                let elements = Arc::new(self.dedup_value_vec(elements.as_ref().clone()));
                Value::Seq(self.dedup_seq(elements))
            }
            Value::Map(object) => {
                let KV(k, v) = object.as_ref();
                let k = Arc::new(self.dedup_value_vec(k.as_ref().clone()));
                let v = self.dedup_value_vec(v.clone());
                let k = self.dedup_seq(k);
                let object = Arc::new(KV(k, v));
                Value::Map(self.dedup_map(object))
            }
            x => x,
        }
    }
}

impl Display for Dedup {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        // writeln!(
        //     f,
        //     "blobs:{}",
        //     DisplayableVec(
        //         &self
        //             .blobs
        //             .iter()
        //             .map(|x| format!("{}:{}", DisplayableBlob(x), Arc::strong_count(x)))
        //             .collect()
        //     )
        // )?;
        writeln!(
            f,
            "strings:{}",
            DisplayableVec(
                &self
                    .strings
                    .iter()
                    .filter(|x| Arc::strong_count(x) > 100)
                    .map(|x| format!("{}:{}\n", x, Arc::strong_count(x)))
                    .collect()
            )
        )?;
        writeln!(
            f,
            "vectors:{}",
            DisplayableVec(
                &self
                    .vectors
                    .iter()
                    .filter(|x| Arc::strong_count(x) > 100)
                    .map(|x| format!("{}:{}\n", DisplayableVec(x), Arc::strong_count(x)))
                    .collect()
            )
        )?;
        writeln!(
            f,
            "objects:{}",
            DisplayableVec(
                &self
                    .objects
                    .iter()
                    .filter(|x| Arc::strong_count(x) > 100)
                    .map(|x| format!("{}:{}\n", DisplayableMap(&x.0, &x.1), Arc::strong_count(x)))
                    .collect()
            )
        )
    }
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, PartialOrd, Ord)]
pub struct KV(Arc<Vec<Value>>, Vec<Value>);

impl KV {
    fn iter(&self) -> impl Iterator<Item = (Value, Value)> {
        self.0
            .as_ref()
            .clone()
            .into_iter()
            .zip(self.1.clone().into_iter())
    }
    fn as_map(&self) -> BTreeMap<Value, Value> {
        self.iter().collect()
    }
}

impl Value {
    fn seq(value: Vec<Value>) -> Value {
        Value::Seq(Arc::new(value))
    }

    fn map(value: BTreeMap<Value, Value>) -> Value {
        let keys: Vec<Value> = value.keys().cloned().collect();
        let values: Vec<Value> = value.values().cloned().collect();
        Value::Map(Arc::new(KV(Arc::new(keys), values)))
    }

    fn string(value: String) -> Value {
        Value::String(Arc::new(value))
    }

    fn bytes(value: Vec<u8>) -> Value {
        Value::Bytes(Arc::new(value))
    }
}

struct DisplayableBlob<'a>(&'a Vec<u8>);

impl Display for DisplayableBlob<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        for byte in self.0 {
            // Decide if you want to pad out the value here
            write!(f, "{:X} ", byte)?;
        }
        Ok(())
    }
}

struct DisplayableVec<'a, T>(&'a Vec<T>);

impl<T: Display> Display for DisplayableVec<'_, T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[")?;
        for (i, elem) in self.0.iter().enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }
            write!(f, "{}", elem)?;
        }
        write!(f, "]")
    }
}

struct DisplayableMap<'a, K, V>(&'a Vec<K>, &'a Vec<V>);

impl<K: Display, V: Display> Display for DisplayableMap<'_, K, V> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{{")?;
        for (i, (k, v)) in self.0.iter().zip(self.1.iter()).enumerate() {
            if i > 0 {
                write!(f, ",")?;
            }
            write!(f, "{}:{}", k, v)?;
        }
        write!(f, "}}")
    }
}

impl Display for Value {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {
            Value::Unit => write!(f, "()"),
            Value::Bool(v) => write!(f, "{}", v),
            Value::U8(v) => write!(f, "{}", v),
            Value::U16(v) => write!(f, "{}", v),
            Value::U32(v) => write!(f, "{}", v),
            Value::U64(v) => write!(f, "{}", v),
            Value::I8(v) => write!(f, "{}", v),
            Value::I16(v) => write!(f, "{}", v),
            Value::I32(v) => write!(f, "{}", v),
            Value::I64(v) => write!(f, "{}", v),
            Value::F32(v) => write!(f, "{}", v),
            Value::F64(v) => write!(f, "{}", v),
            Value::Char(v) => write!(f, "{}", v),
            Value::String(ref v) => write!(f, "{}", v),
            Value::Bytes(ref v) => write!(f, "{:?}", v),
            Value::Option(ref v) => v
                .clone()
                .map(|v| write!(f, "Some({})", v))
                .unwrap_or_else(|| write!(f, "None")),
            Value::Newtype(ref v) => write!(f, "{}", v),
            Value::Seq(ref v) => write!(f, "{}", DisplayableVec(v)),
            Value::Map(ref v) => write!(f, "{}", DisplayableMap(&v.0, &v.1)),
        }
    }
}

impl Hash for Value {
    fn hash<H>(&self, hasher: &mut H)
    where
        H: Hasher,
    {
        self.discriminant().hash(hasher);
        match *self {
            Value::Bool(v) => v.hash(hasher),
            Value::U8(v) => v.hash(hasher),
            Value::U16(v) => v.hash(hasher),
            Value::U32(v) => v.hash(hasher),
            Value::U64(v) => v.hash(hasher),
            Value::I8(v) => v.hash(hasher),
            Value::I16(v) => v.hash(hasher),
            Value::I32(v) => v.hash(hasher),
            Value::I64(v) => v.hash(hasher),
            Value::F32(v) => OrderedFloat(v).hash(hasher),
            Value::F64(v) => OrderedFloat(v).hash(hasher),
            Value::Char(v) => v.hash(hasher),
            Value::String(ref v) => v.hash(hasher),
            Value::Unit => ().hash(hasher),
            Value::Option(ref v) => v.hash(hasher),
            Value::Newtype(ref v) => v.hash(hasher),
            Value::Seq(ref v) => v.hash(hasher),
            Value::Map(ref v) => v.hash(hasher),
            Value::Bytes(ref v) => v.hash(hasher),
        }
    }
}

impl PartialEq for Value {
    fn eq(&self, rhs: &Self) -> bool {
        match (self, rhs) {
            (&Value::Bool(v0), &Value::Bool(v1)) => v0 == v1,
            (&Value::U8(v0), &Value::U8(v1)) => v0 == v1,
            (&Value::U16(v0), &Value::U16(v1)) => v0 == v1,
            (&Value::U32(v0), &Value::U32(v1)) => v0 == v1,
            (&Value::U64(v0), &Value::U64(v1)) => v0 == v1,
            (&Value::I8(v0), &Value::I8(v1)) => v0 == v1,
            (&Value::I16(v0), &Value::I16(v1)) => v0 == v1,
            (&Value::I32(v0), &Value::I32(v1)) => v0 == v1,
            (&Value::I64(v0), &Value::I64(v1)) => v0 == v1,
            (&Value::F32(v0), &Value::F32(v1)) => OrderedFloat(v0) == OrderedFloat(v1),
            (&Value::F64(v0), &Value::F64(v1)) => OrderedFloat(v0) == OrderedFloat(v1),
            (&Value::Char(v0), &Value::Char(v1)) => v0 == v1,
            (&Value::String(ref v0), &Value::String(ref v1)) => v0 == v1,
            (&Value::Unit, &Value::Unit) => true,
            (&Value::Option(ref v0), &Value::Option(ref v1)) => v0 == v1,
            (&Value::Newtype(ref v0), &Value::Newtype(ref v1)) => v0 == v1,
            (&Value::Seq(ref v0), &Value::Seq(ref v1)) => v0 == v1,
            (&Value::Map(ref v0), &Value::Map(ref v1)) => v0 == v1,
            (&Value::Bytes(ref v0), &Value::Bytes(ref v1)) => v0 == v1,
            _ => false,
        }
    }
}

impl Ord for Value {
    fn cmp(&self, rhs: &Self) -> Ordering {
        match (self, rhs) {
            (&Value::Bool(v0), &Value::Bool(ref v1)) => v0.cmp(v1),
            (&Value::U8(v0), &Value::U8(ref v1)) => v0.cmp(v1),
            (&Value::U16(v0), &Value::U16(ref v1)) => v0.cmp(v1),
            (&Value::U32(v0), &Value::U32(ref v1)) => v0.cmp(v1),
            (&Value::U64(v0), &Value::U64(ref v1)) => v0.cmp(v1),
            (&Value::I8(v0), &Value::I8(ref v1)) => v0.cmp(v1),
            (&Value::I16(v0), &Value::I16(ref v1)) => v0.cmp(v1),
            (&Value::I32(v0), &Value::I32(ref v1)) => v0.cmp(v1),
            (&Value::I64(v0), &Value::I64(ref v1)) => v0.cmp(v1),
            (&Value::F32(v0), &Value::F32(v1)) => OrderedFloat(v0).cmp(&OrderedFloat(v1)),
            (&Value::F64(v0), &Value::F64(v1)) => OrderedFloat(v0).cmp(&OrderedFloat(v1)),
            (&Value::Char(v0), &Value::Char(ref v1)) => v0.cmp(v1),
            (&Value::String(ref v0), &Value::String(ref v1)) => v0.cmp(v1),
            (&Value::Unit, &Value::Unit) => Ordering::Equal,
            (&Value::Option(ref v0), &Value::Option(ref v1)) => v0.cmp(v1),
            (&Value::Newtype(ref v0), &Value::Newtype(ref v1)) => v0.cmp(v1),
            (&Value::Seq(ref v0), &Value::Seq(ref v1)) => v0.cmp(v1),
            (&Value::Map(ref v0), &Value::Map(ref v1)) => v0.cmp(v1),
            (&Value::Bytes(ref v0), &Value::Bytes(ref v1)) => v0.cmp(v1),
            (ref v0, ref v1) => v0.discriminant().cmp(&v1.discriminant()),
        }
    }
}

impl Value {
    fn discriminant(&self) -> usize {
        match *self {
            Value::Bool(..) => 0,
            Value::U8(..) => 1,
            Value::U16(..) => 2,
            Value::U32(..) => 3,
            Value::U64(..) => 4,
            Value::I8(..) => 5,
            Value::I16(..) => 6,
            Value::I32(..) => 7,
            Value::I64(..) => 8,
            Value::F32(..) => 9,
            Value::F64(..) => 10,
            Value::Char(..) => 11,
            Value::String(..) => 12,
            Value::Unit => 13,
            Value::Option(..) => 14,
            Value::Newtype(..) => 15,
            Value::Seq(..) => 16,
            Value::Map(..) => 17,
            Value::Bytes(..) => 18,
        }
    }

    fn unexpected(&self) -> serde::de::Unexpected {
        match *self {
            Value::Bool(b) => serde::de::Unexpected::Bool(b),
            Value::U8(n) => serde::de::Unexpected::Unsigned(n as u64),
            Value::U16(n) => serde::de::Unexpected::Unsigned(n as u64),
            Value::U32(n) => serde::de::Unexpected::Unsigned(n as u64),
            Value::U64(n) => serde::de::Unexpected::Unsigned(n),
            Value::I8(n) => serde::de::Unexpected::Signed(n as i64),
            Value::I16(n) => serde::de::Unexpected::Signed(n as i64),
            Value::I32(n) => serde::de::Unexpected::Signed(n as i64),
            Value::I64(n) => serde::de::Unexpected::Signed(n),
            Value::F32(n) => serde::de::Unexpected::Float(n as f64),
            Value::F64(n) => serde::de::Unexpected::Float(n),
            Value::Char(c) => serde::de::Unexpected::Char(c),
            Value::String(ref s) => serde::de::Unexpected::Str(s),
            Value::Unit => serde::de::Unexpected::Unit,
            Value::Option(_) => serde::de::Unexpected::Option,
            Value::Newtype(_) => serde::de::Unexpected::NewtypeStruct,
            Value::Seq(_) => serde::de::Unexpected::Seq,
            Value::Map(_) => serde::de::Unexpected::Map,
            Value::Bytes(ref b) => serde::de::Unexpected::Bytes(b),
        }
    }

    pub fn deserialize_into<'de, T: Deserialize<'de>>(self) -> Result<T, DeserializerError> {
        T::deserialize(self)
    }
}

impl Eq for Value {}
impl PartialOrd for Value {
    fn partial_cmp(&self, rhs: &Self) -> Option<Ordering> {
        Some(self.cmp(rhs))
    }
}

#[test]
fn de_smoke_test() {
    // some convoluted Value
    let value = Value::Option(Some(Box::new(Value::seq(vec![
        Value::U16(8),
        Value::Char('a'),
        Value::F32(1.0),
        Value::string("hello".into()),
        Value::map(
            vec![
                (Value::Bool(false), Value::Unit),
                (
                    Value::Bool(true),
                    Value::Newtype(Box::new(Value::bytes(b"hi".as_ref().into()))),
                ),
            ]
            .into_iter()
            .collect(),
        ),
    ]))));

    // assert that the value remains unchanged through deserialization
    let value_de = Value::deserialize(value.clone()).unwrap();
    assert_eq!(value_de, value);
}

#[test]
fn ser_smoke_test() {
    #[derive(Serialize)]
    struct Foo {
        a: u32,
        b: String,
        c: Vec<bool>,
    }

    let foo = Foo {
        a: 15,
        b: "hello".into(),
        c: vec![true, false],
    };

    let expected = Value::map(
        vec![
            (Value::string("a".into()), Value::U32(15)),
            (Value::string("b".into()), Value::string("hello".into())),
            (
                Value::string("c".into()),
                Value::seq(vec![Value::Bool(true), Value::Bool(false)]),
            ),
        ]
        .into_iter()
        .collect(),
    );

    let value = to_value(&foo).unwrap();
    assert_eq!(expected, value);
}

#[test]
fn deserialize_into_enum() {
    #[derive(Deserialize, Debug, PartialEq, Eq)]
    enum Foo {
        Bar,
        Baz(u8),
    }

    let value = Value::string("Bar".into());
    assert_eq!(Foo::deserialize(value).unwrap(), Foo::Bar);

    let value = Value::map(
        vec![(Value::string("Baz".into()), Value::U8(1))]
            .into_iter()
            .collect(),
    );
    assert_eq!(Foo::deserialize(value).unwrap(), Foo::Baz(1));
}

#[test]
fn deserialize_inside_deserialize_impl() {
    #[derive(Debug, PartialEq, Eq)]
    enum Event {
        Added(u32),
        Error(u8),
    }

    impl<'de> serde::Deserialize<'de> for Event {
        fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
        where
            D: serde::Deserializer<'de>,
        {
            #[derive(Deserialize)]
            struct RawEvent {
                kind: String,
                object: Value,
            }

            let raw_event = RawEvent::deserialize(deserializer)?;

            // Cannot directly use Value as Deserializer, since error type needs to be
            // generic D::Error rather than specific serde_value::DeserializerError
            let object_deserializer = ValueDeserializer::new(raw_event.object);

            Ok(match &*raw_event.kind {
                "ADDED" => Event::Added(<_>::deserialize(object_deserializer)?),
                "ERROR" => Event::Error(<_>::deserialize(object_deserializer)?),
                kind => return Err(serde::de::Error::unknown_variant(kind, &["ADDED", "ERROR"])),
            })
        }
    }

    let input = Value::map(
        vec![
            (
                Value::string("kind".to_owned()),
                Value::string("ADDED".to_owned()),
            ),
            (Value::string("object".to_owned()), Value::U32(5)),
        ]
        .into_iter()
        .collect(),
    );
    let event = Event::deserialize(input).expect("could not deserialize ADDED event");
    assert_eq!(event, Event::Added(5));

    let input = Value::map(
        vec![
            (
                Value::string("kind".to_owned()),
                Value::string("ERROR".to_owned()),
            ),
            (Value::string("object".to_owned()), Value::U8(5)),
        ]
        .into_iter()
        .collect(),
    );
    let event = Event::deserialize(input).expect("could not deserialize ERROR event");
    assert_eq!(event, Event::Error(5));

    let input = Value::map(
        vec![
            (
                Value::string("kind".to_owned()),
                Value::string("ADDED".to_owned()),
            ),
            (Value::string("object".to_owned()), Value::Unit),
        ]
        .into_iter()
        .collect(),
    );
    let _ = Event::deserialize(input).expect_err("expected deserializing bad ADDED event to fail");
}

#[test]
fn deserialize_newtype() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Foo(i32);

    let input = Value::I32(5);
    let foo = Foo::deserialize(input).unwrap();
    assert_eq!(foo, Foo(5));
}

#[test]
fn deserialize_newtype2() {
    #[derive(Debug, Deserialize, PartialEq)]
    struct Foo(i32);

    #[derive(Debug, Deserialize, PartialEq)]
    struct Bar {
        foo: Foo,
    }

    let input = Value::map(
        vec![(Value::string("foo".to_owned()), Value::I32(5))]
            .into_iter()
            .collect(),
    );
    let bar = Bar::deserialize(input).unwrap();
    assert_eq!(bar, Bar { foo: Foo(5) });
}

#[cfg(test)]
mod dedup_tests {
    extern crate serde_json;

    use self::serde_json::json;
    use super::*;

    #[test]
    fn dedup_simple() {
        let input = Value::seq(vec![
            Value::string("a".to_owned()),
            Value::string("a".to_owned()),
        ]);
        let mut dedup = Dedup::new();
        let result = dedup.dedup(input);
        if let Value::Seq(x) = result {
            if let Value::String(ref a) = x[0] {
                if let Value::String(ref b) = x[1] {
                    assert!(Arc::ptr_eq(a, b));
                } else {
                    panic!();
                }
            } else {
                panic!();
            }
        } else {
            panic!();
        }
    }

    #[test]
    fn dedup_record() {
        let input = json!(
            [{ "x": 0, "y":0},{ "x": 0, "y":1},{ "x": 1, "y":1},{ "x": 1, "y":0}]
        );
        let value: Value = to_value(input).unwrap();
        let mut dedup = Dedup::new();
        let result = dedup.dedup(value);
        println!("{}", dedup);
        println!("{}", result);

        let mut strings: Vec<&str> = dedup.strings.iter().map(|x| x.as_ref().as_ref()).collect();
        strings.sort();
        assert_eq!(strings, vec!["x", "y"]);

        if let Value::Seq(x) = result {
            if let Value::Map(ref a) = x[0] {
                if let Value::Map(ref b) = x[1] {
                    assert!(Arc::ptr_eq(&a.as_ref().0, &b.as_ref().0));
                } else {
                    panic!();
                }
            } else {
                panic!();
            }
        } else {
            panic!();
        }
    }

    use std::io::BufRead;
    #[test]
    fn dedup_large() {
        println!("sov {}", std::mem::size_of::<Value>());
        let f = std::fs::File::open("large.json").unwrap();
        let f = std::io::BufReader::new(f);
        let mut dedup = Dedup::new();
        let lines: Vec<Value> = f.lines().enumerate().map(|(i, x)| {
            let text = x.unwrap();
            let json: serde_json::Value = serde_json::from_str(&text).unwrap();
            let value = to_value(json).unwrap();
            let value = dedup.dedup(value);
            if (i % 1000) == 0 {
                println!("{}", i);
            }
            value
        }).collect();
        drop(lines);
        let mut strings = dedup.strings();
        strings = strings.iter().cloned().filter(|x| x.1 > 10).collect::<Vec<_>>();
        strings.sort_by_key(|x| x.1);
        println!("{:?}", strings);
        println!("{}", dedup.size());
//        println!("{}", dedup);
    }    
}
