//! [`Native`]: types `#[probe]` passes to the tracer as one 64-bit value.

/// A type `#[probe]` can pass to the tracer as one unsigned 64-bit value,
/// with `native(arg)`.
///
/// `#[probe]` recognizes the primitive types by how they are written. A type
/// alias or a newtype is not recognized, so it needs `native(arg)`, and its
/// type must implement this trait. The tracer sees the value as an unsigned
/// 64-bit integer: a negative signed value appears in two's complement, and
/// a pointer as its address.
///
/// ```
/// #[derive(Clone, Copy)]
/// struct RowId(u32);
///
/// impl anyprobe::Native for RowId {
///     fn to_u64(&self) -> u64 {
///         u64::from(self.0)
///     }
/// }
///
/// #[anyprobe::probe(native(row))]
/// fn fetch(row: RowId) {}
/// # fetch(RowId(1));
/// ```
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be passed to a probe as a native value",
    label = "`{Self}` does not implement `anyprobe::Native`",
    note = "implement `anyprobe::Native` for it, or list the argument in `debug(..)` or `serde(..)` instead of `native(..)`"
)]
pub trait Native {
    /// The value passed to the tracer.
    fn to_u64(&self) -> u64;
}

macro_rules! unsigned {
    ($($t:ty),*) => {$(
        impl Native for $t {
            #[inline(always)]
            fn to_u64(&self) -> u64 {
                *self as u64
            }
        }
    )*};
}

macro_rules! signed {
    ($($t:ty),*) => {$(
        impl Native for $t {
            /// Sign-extended to 64 bits, then reinterpreted as unsigned.
            #[inline(always)]
            fn to_u64(&self) -> u64 {
                *self as i64 as u64
            }
        }
    )*};
}

unsigned!(u8, u16, u32, u64, usize);
signed!(i8, i16, i32, i64, isize);

impl Native for bool {
    #[inline(always)]
    fn to_u64(&self) -> u64 {
        u64::from(*self)
    }
}

impl Native for char {
    #[inline(always)]
    fn to_u64(&self) -> u64 {
        u64::from(*self)
    }
}

impl<T: ?Sized> Native for *const T {
    #[inline(always)]
    fn to_u64(&self) -> u64 {
        self.cast::<()>() as usize as u64
    }
}

impl<T: ?Sized> Native for *mut T {
    #[inline(always)]
    fn to_u64(&self) -> u64 {
        self.cast::<()>() as usize as u64
    }
}

impl<T: Native + ?Sized> Native for &T {
    #[inline(always)]
    fn to_u64(&self) -> u64 {
        (**self).to_u64()
    }
}

impl<T: Native + ?Sized> Native for &mut T {
    #[inline(always)]
    fn to_u64(&self) -> u64 {
        (**self).to_u64()
    }
}
