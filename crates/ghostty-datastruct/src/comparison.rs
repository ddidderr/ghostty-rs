//! Deep/approximate equality helpers (`comparison.zig`).
//!
//! The Zig version uses comptime reflection to compare arbitrary types
//! field-by-field. In Rust, `PartialEq` already covers structural equality;
//! [`DeepEqual`] exists to allow a type to customise its "deep equal"
//! behaviour independent of `PartialEq` (the Zig `equal` decl), and
//! [`ApproxEq`] provides the float-tolerant comparison used by
//! `expectApproxEqual`.

/// A deep equality comparison that works for most types. Types may implement
/// this to customise comparison (the Zig `equal` declaration). Blanket
/// implementations exist for primitives, strings, and containers of
/// `DeepEqual` types.
pub trait DeepEqual {
    fn deep_equal(&self, other: &Self) -> bool;
}

/// Compare two values with [`DeepEqual`].
pub fn deep_equal<T: DeepEqual + ?Sized>(old: &T, new: &T) -> bool {
    old.deep_equal(new)
}

macro_rules! impl_deep_equal_prim {
    ($($t:ty),* $(,)?) => {
        $(impl DeepEqual for $t {
            #[allow(clippy::float_cmp)]
            fn deep_equal(&self, other: &Self) -> bool { self == other }
        })*
    };
}

impl_deep_equal_prim!(
    (),
    bool,
    char,
    i8,
    i16,
    i32,
    i64,
    i128,
    isize,
    u8,
    u16,
    u32,
    u64,
    u128,
    usize,
    f32,
    f64,
    str,
    String,
);

impl<T: DeepEqual> DeepEqual for Option<T> {
    fn deep_equal(&self, other: &Self) -> bool {
        match (self, other) {
            (None, None) => true,
            (Some(a), Some(b)) => a.deep_equal(b),
            _ => false,
        }
    }
}

impl<T: DeepEqual> DeepEqual for [T] {
    fn deep_equal(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a.deep_equal(b))
    }
}

impl<T: DeepEqual, const N: usize> DeepEqual for [T; N] {
    fn deep_equal(&self, other: &Self) -> bool {
        self.as_slice().deep_equal(other.as_slice())
    }
}

impl<T: DeepEqual> DeepEqual for Vec<T> {
    fn deep_equal(&self, other: &Self) -> bool {
        self.as_slice().deep_equal(other.as_slice())
    }
}

impl<T: DeepEqual + ?Sized> DeepEqual for &T {
    fn deep_equal(&self, other: &Self) -> bool {
        (**self).deep_equal(*other)
    }
}

/// Approximate equality: relative tolerance of `sqrt(eps)` for floats and
/// plain equality for everything else.
pub trait ApproxEq {
    fn approx_eq(&self, other: &Self) -> bool;
}

macro_rules! impl_approx_float {
    ($($t:ty),*) => {
        $(impl ApproxEq for $t {
            fn approx_eq(&self, other: &Self) -> bool {
                // std.math.approxEqRel with tolerance sqrt(eps)
                let tolerance = <$t>::EPSILON.sqrt();
                let (a, b) = (*self, *other);
                if a == b { return true; }
                if a.is_nan() || b.is_nan() { return false; }
                if a.is_infinite() || b.is_infinite() { return false; }
                (a - b).abs() <= a.abs().max(b.abs()) * tolerance
            }
        })*
    };
}
impl_approx_float!(f32, f64);

macro_rules! impl_approx_eq_prim {
    ($($t:ty),* $(,)?) => {
        $(impl ApproxEq for $t {
            fn approx_eq(&self, other: &Self) -> bool { self == other }
        })*
    };
}
impl_approx_eq_prim!(
    (),
    bool,
    char,
    i8,
    i16,
    i32,
    i64,
    isize,
    u8,
    u16,
    u32,
    u64,
    usize,
    str,
    String
);

impl<T: ApproxEq> ApproxEq for Option<T> {
    fn approx_eq(&self, other: &Self) -> bool {
        match (self, other) {
            (None, None) => true,
            (Some(a), Some(b)) => a.approx_eq(b),
            _ => false,
        }
    }
}

impl<T: ApproxEq> ApproxEq for [T] {
    fn approx_eq(&self, other: &Self) -> bool {
        self.len() == other.len() && self.iter().zip(other).all(|(a, b)| a.approx_eq(b))
    }
}

impl<T: ApproxEq, const N: usize> ApproxEq for [T; N] {
    fn approx_eq(&self, other: &Self) -> bool {
        self.as_slice().approx_eq(other.as_slice())
    }
}

impl<T: ApproxEq> ApproxEq for Vec<T> {
    fn approx_eq(&self, other: &Self) -> bool {
        self.as_slice().approx_eq(other.as_slice())
    }
}

macro_rules! impl_approx_tuple {
    ($(($($n:ident : $i:tt),+)),+ $(,)?) => {
        $(impl<$($n: ApproxEq),+> ApproxEq for ($($n,)+) {
            fn approx_eq(&self, other: &Self) -> bool {
                $(self.$i.approx_eq(&other.$i))&&+
            }
        })+
    };
}
impl_approx_tuple!((A: 0), (A: 0, B: 1), (A: 0, B: 1, C: 2), (A: 0, B: 1, C: 2, D: 3));

/// Asserts that two values are (approximately) equal, panicking otherwise.
/// Counterpart of `expectApproxEqual`.
#[track_caller]
pub fn expect_approx_equal<T: ApproxEq + std::fmt::Debug + ?Sized>(expected: &T, actual: &T) {
    assert!(
        expected.approx_eq(actual),
        "expected approximately {expected:?}, found {actual:?}"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq)]
    enum Value {
        Int(i32),
        Float(f32),
    }

    impl ApproxEq for Value {
        fn approx_eq(&self, other: &Self) -> bool {
            match (self, other) {
                (Value::Int(a), Value::Int(b)) => a.approx_eq(b),
                (Value::Float(a), Value::Float(b)) => a.approx_eq(b),
                _ => false,
            }
        }
    }

    #[test]
    fn expect_approx_equal_union_enum() {
        let b10 = Value::Float(10.0);
        let b10plus = Value::Float(10.000001);
        expect_approx_equal(&b10, &b10plus);
        assert!(!Value::Int(1).approx_eq(&Value::Float(1.0)));
        assert!(!Value::Float(10.0).approx_eq(&Value::Float(10.1)));
    }

    #[test]
    fn expect_approx_equal_nested_array() {
        let a = [[1.0f32, 0.0], [0.0, 1.0]];
        let b = [[1.000001f32, 0.0], [0.0, 0.999999]];
        expect_approx_equal(&a, &b);
        let c = [[1.1f32, 0.0], [0.0, 1.0]];
        assert!(!a.approx_eq(&c));
    }

    #[test]
    fn expect_approx_equal_vector() {
        // Zig @Vector(4, f32) maps to a fixed array.
        let a = [4.0f32; 4];
        let b = [4.000001f32; 4];
        expect_approx_equal(&a, &b);
    }

    #[test]
    fn expect_approx_equal_struct() {
        // Zig anonymous struct maps to a tuple.
        let a = (1, 1.0f32);
        let b = (1, 0.999999f32);
        expect_approx_equal(&a, &b);
    }

    #[test]
    fn deep_equal_void() {
        assert!(deep_equal(&(), &()));
    }

    #[test]
    fn deep_equal_bool() {
        assert!(deep_equal(&true, &true));
        assert!(deep_equal(&false, &false));
        assert!(!deep_equal(&true, &false));
        assert!(!deep_equal(&false, &true));
    }

    #[test]
    fn deep_equal_int() {
        assert!(deep_equal(&42i32, &42));
        assert!(deep_equal(&-100i32, &-100));
        assert!(!deep_equal(&42i32, &43));
        assert!(deep_equal(&0u64, &0));
        assert!(!deep_equal(&0u64, &1));
    }

    #[test]
    fn deep_equal_float() {
        assert!(deep_equal(&1.0f32, &1.0));
        assert!(!deep_equal(&1.0f32, &1.1));
        assert!(deep_equal(&1.23456f64, &1.23456));
        assert!(!deep_equal(&1.23456f64, &1.23455));
    }

    #[test]
    fn deep_equal_enum() {
        #[derive(PartialEq)]
        enum Color {
            Red,
            Green,
            Blue,
        }
        impl DeepEqual for Color {
            fn deep_equal(&self, other: &Self) -> bool {
                self == other
            }
        }
        assert!(deep_equal(&Color::Red, &Color::Red));
        assert!(deep_equal(&Color::Blue, &Color::Blue));
        assert!(!deep_equal(&Color::Red, &Color::Green));
        assert!(!deep_equal(&Color::Green, &Color::Blue));
    }

    #[test]
    fn deep_equal_slice_u8() {
        assert!(deep_equal("hello", "hello"));
        assert!(deep_equal("", ""));
        assert!(!deep_equal("hello", "world"));
        assert!(!deep_equal("hello", "hell"));
        assert!(!deep_equal("hello", "hello!"));
        assert!(deep_equal(b"hello".as_slice(), b"hello".as_slice()));
    }

    #[test]
    fn deep_equal_sentinel_slice_u8() {
        // [:0]const u8 maps to &str / CString-like; plain strings here.
        assert!(deep_equal("foo", "foo"));
        assert!(!deep_equal("foo", "bar"));
        assert!(!deep_equal("foo", "fo"));
    }

    #[test]
    fn deep_equal_slice_of_strings() {
        let a: &[&str] = &["one", "two", "three"];
        let b: &[&str] = &["one", "two", "three"];
        let c: &[&str] = &["one", "two"];
        let d: &[&str] = &["one", "two", "four"];
        let e: &[&str] = &[];

        assert!(deep_equal(a, b));
        assert!(!deep_equal(a, c));
        assert!(!deep_equal(a, d));
        assert!(deep_equal(e, e));
        assert!(!deep_equal(a, e));
    }

    #[test]
    fn deep_equal_optional() {
        assert!(deep_equal::<Option<i32>>(&None, &None));
        assert!(deep_equal(&Some(42), &Some(42)));
        assert!(!deep_equal(&None, &Some(42)));
        assert!(!deep_equal(&Some(42), &None));
        assert!(!deep_equal(&Some(42), &Some(43)));
    }

    #[derive(Debug, PartialEq)]
    struct Nested {
        x: i32,
        y: i32,
    }
    impl DeepEqual for Nested {
        fn deep_equal(&self, other: &Self) -> bool {
            self.x.deep_equal(&other.x) && self.y.deep_equal(&other.y)
        }
    }

    #[test]
    fn deep_equal_optional_nested() {
        assert!(deep_equal::<Option<Nested>>(&None, &None));
        assert!(deep_equal(
            &Some(Nested { x: 1, y: 2 }),
            &Some(Nested { x: 1, y: 2 })
        ));
        assert!(!deep_equal(
            &Some(Nested { x: 1, y: 2 }),
            &Some(Nested { x: 1, y: 3 })
        ));
        assert!(!deep_equal(&Some(Nested { x: 1, y: 2 }), &None));
    }

    #[test]
    fn deep_equal_array() {
        assert!(deep_equal(&[1, 2, 3], &[1, 2, 3]));
        assert!(!deep_equal(&[1, 2, 3], &[1, 2, 4]));
        assert!(!deep_equal(&[1, 2, 3], &[0, 2, 3]));
        assert!(deep_equal::<[i32; 0]>(&[], &[]));
    }

    #[test]
    fn deep_equal_nested_array() {
        let a = [[1, 2], [3, 4]];
        let b = [[1, 2], [3, 4]];
        let c = [[1, 2], [3, 5]];
        assert!(deep_equal(&a, &b));
        assert!(!deep_equal(&a, &c));
    }

    struct Outer {
        a: Nested,
        b: Nested,
    }
    impl DeepEqual for Outer {
        fn deep_equal(&self, o: &Self) -> bool {
            self.a.deep_equal(&o.a) && self.b.deep_equal(&o.b)
        }
    }

    #[test]
    fn deep_equal_struct() {
        assert!(deep_equal(
            &Nested { x: 10, y: 20 },
            &Nested { x: 10, y: 20 }
        ));
        assert!(!deep_equal(
            &Nested { x: 10, y: 20 },
            &Nested { x: 10, y: 21 }
        ));
        assert!(!deep_equal(
            &Nested { x: 10, y: 20 },
            &Nested { x: 11, y: 20 }
        ));
    }

    #[test]
    fn deep_equal_struct_nested() {
        let mk = |a, b| Outer {
            a: Nested { x: a, y: 0 },
            b: Nested { x: b, y: 0 },
        };
        assert!(deep_equal(&mk(1, 2), &mk(1, 2)));
        assert!(!deep_equal(&mk(1, 2), &mk(1, 3)));
    }

    struct Custom {
        value: i32,
    }
    impl DeepEqual for Custom {
        fn deep_equal(&self, other: &Self) -> bool {
            self.value.rem_euclid(10) == other.value.rem_euclid(10)
        }
    }

    #[test]
    fn deep_equal_struct_with_equal_decl() {
        assert!(deep_equal(&Custom { value: 5 }, &Custom { value: 15 }));
        assert!(deep_equal(&Custom { value: 100 }, &Custom { value: 0 }));
        assert!(!deep_equal(&Custom { value: 5 }, &Custom { value: 6 }));
    }

    #[derive(Debug)]
    enum U {
        Int(i32),
        Float(f32),
        None,
    }
    impl DeepEqual for U {
        fn deep_equal(&self, other: &Self) -> bool {
            match (self, other) {
                (U::Int(a), U::Int(b)) => a == b,
                (U::Float(a), U::Float(b)) => a == b,
                (U::None, U::None) => true,
                _ => false,
            }
        }
    }

    #[test]
    fn deep_equal_union() {
        assert!(deep_equal(&U::Int(42), &U::Int(42)));
        assert!(!deep_equal(&U::Int(42), &U::Int(43)));
        assert!(!deep_equal(&U::Int(42), &U::Float(42.0)));
        assert!(deep_equal(&U::None, &U::None));
        assert!(!deep_equal(&U::None, &U::Int(0)));
    }

    enum UCustom {
        Num(i32),
        Str(&'static str),
    }
    impl DeepEqual for UCustom {
        fn deep_equal(&self, other: &Self) -> bool {
            match (self, other) {
                (UCustom::Num(n), UCustom::Num(m)) => n.rem_euclid(10) == m.rem_euclid(10),
                (UCustom::Str(s), UCustom::Str(t)) => s.len() == t.len(),
                _ => false,
            }
        }
    }

    #[test]
    fn deep_equal_union_with_equal_decl() {
        assert!(deep_equal(&UCustom::Num(5), &UCustom::Num(25)));
        assert!(!deep_equal(&UCustom::Num(5), &UCustom::Num(6)));
        assert!(deep_equal(&UCustom::Str("abc"), &UCustom::Str("xyz")));
        assert!(!deep_equal(&UCustom::Str("abc"), &UCustom::Str("ab")));
    }

    struct Item {
        id: i32,
        name: &'static str,
    }
    impl DeepEqual for Item {
        fn deep_equal(&self, o: &Self) -> bool {
            self.id.deep_equal(&o.id) && self.name.deep_equal(o.name)
        }
    }

    #[test]
    fn deep_equal_array_of_structs() {
        let mk = |n: &'static str| [Item { id: 1, name: "one" }, Item { id: 2, name: n }];
        assert!(deep_equal(&mk("two"), &mk("two")));
        assert!(!deep_equal(&mk("two"), &mk("TWO")));
    }

    struct Config {
        name: &'static str,
        port: Option<u16>,
    }
    impl DeepEqual for Config {
        fn deep_equal(&self, o: &Self) -> bool {
            self.name.deep_equal(o.name) && self.port.deep_equal(&o.port)
        }
    }

    #[test]
    fn deep_equal_struct_with_optional_field() {
        let c = |port| Config { name: "app", port };
        assert!(deep_equal(&c(Some(8080)), &c(Some(8080))));
        assert!(deep_equal(&c(None), &c(None)));
        assert!(!deep_equal(&c(Some(8080)), &c(None)));
        assert!(!deep_equal(&c(Some(8080)), &c(Some(8081))));
    }

    struct Data {
        values: [i32; 3],
    }
    impl DeepEqual for Data {
        fn deep_equal(&self, o: &Self) -> bool {
            self.values.deep_equal(&o.values)
        }
    }

    #[test]
    fn deep_equal_struct_with_array_field() {
        assert!(deep_equal(
            &Data { values: [1, 2, 3] },
            &Data { values: [1, 2, 3] }
        ));
        assert!(!deep_equal(
            &Data { values: [1, 2, 3] },
            &Data { values: [1, 2, 4] }
        ));
    }

    #[test]
    fn deep_equal_vec() {
        assert!(deep_equal(&vec![1, 2], &vec![1, 2]));
        assert!(!deep_equal(&vec![1, 2], &vec![1]));
    }
}
