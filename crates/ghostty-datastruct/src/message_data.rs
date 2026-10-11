//! Data that fits in an array, is a stable pointer, or is heap allocated
//! (`message_data.zig`). Helpful for thread messaging utilities.

/// A union that can be used to accommodate data that fit within an array,
/// are a stable pointer, or require deallocation. This is helpful for thread
/// messaging utilities.
#[derive(Debug, Clone)]
pub enum MessageData<Elem: Copy + Default + 'static, const SMALL_SIZE: usize> {
    /// A small write where the data fits into this union size.
    Small {
        data: [Elem; SMALL_SIZE],
        len: usize,
    },

    /// A stable pointer so we can just pass the slice directly through.
    /// This is useful i.e. for const data.
    Stable(&'static [Elem]),

    /// Allocated. Freed on drop. This should be rarely used.
    Alloc(Vec<Elem>),
}

impl<Elem: Copy + Default + 'static, const SMALL_SIZE: usize> MessageData<Elem, SMALL_SIZE> {
    /// The maximum number of elements in the small variant.
    pub const MAX: usize = SMALL_SIZE;

    /// Initializes the union for a given data type. This will
    /// attempt to fit into a small value if possible, otherwise
    /// will allocate and put into `Alloc`.
    ///
    /// This can't and will never detect stable pointers.
    #[must_use]
    pub fn init(data: &[Elem]) -> Self {
        // If it fits in our small request, do that.
        if data.len() <= SMALL_SIZE {
            let mut buf = [Elem::default(); SMALL_SIZE];
            buf[..data.len()].copy_from_slice(data);
            return Self::Small {
                data: buf,
                len: data.len(),
            };
        }

        // Otherwise, allocate
        Self::Alloc(data.to_vec())
    }

    /// Returns a const slice of the data pointed to by this request.
    #[must_use]
    pub fn slice(&self) -> &[Elem] {
        match self {
            Self::Small { data, len } => &data[..*len],
            Self::Stable(v) => v,
            Self::Alloc(v) => v,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_data_init_small() {
        let io = MessageData::<u8, 10>::init(b"hello!");
        assert!(matches!(io, MessageData::Small { .. }));
        assert_eq!(io.slice(), b"hello!");
    }

    #[test]
    fn message_data_init_alloc() {
        let input = "hello! ".repeat(100);
        let io = MessageData::<u8, 10>::init(input.as_bytes());
        assert!(matches!(io, MessageData::Alloc(_)));
        assert_eq!(io.slice(), input.as_bytes());
    }

    #[test]
    fn message_data_small_fits_non_u8_sized_data() {
        let input = [b'X'; 500];
        let io = MessageData::<u8, 500>::init(&input);
        assert!(matches!(io, MessageData::Small { .. }));
        assert_eq!(io.slice().len(), 500);
    }

    #[test]
    fn message_data_stable() {
        let io = MessageData::<u8, 4>::Stable(b"static");
        assert_eq!(io.slice(), b"static");
    }
}
