use core::fmt;
use core::ops::{Add, AddAssign, Sub, SubAssign};
use super::{align_up, align_down};
use bit_field::BitField;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct PhysicalAddress(u64);

#[derive(Debug)]
pub struct PhysicalAddressNotValid(u64);

impl PhysicalAddress {
    #[inline]
    pub fn new(addr: u64) -> Self {
        Self::try_new(addr).expect("Invalid physical address")
    }

    #[inline]
    pub fn try_new(addr: u64) -> Result<Self, PhysicalAddressNotValid> {
        match addr.get_bits(52..64) {
            0 => Ok(Self(addr)),                          // Address is valid
            other => Err(PhysicalAddressNotValid(other)), // address is not valid
        }
    }

    #[inline]
    pub const fn new_truncate(addr: u64) -> Self {
        Self(addr % (1 << 52))
    }

    #[inline]
    pub const unsafe fn new_unsafe(addr: u64) -> Self {
        Self(addr)
    }

    #[inline]
    pub const fn zero() -> Self {
        Self(0)
    }

    #[inline]
    pub const fn as_u64(self) -> u64 {
        self.0
    }

    #[cfg(target_pointer_width = "64")]
    #[inline]
    pub fn from_ptr<T>(ptr: *const T) -> Self {
        Self::new(ptr as u64)
    }

    #[cfg(target_pointer_width = "64")]
    #[inline]
    pub fn as_ptr<T>(self) -> *const T {
        self.as_u64() as *const T
    }

    #[cfg(target_pointer_width = "64")]
    #[inline]
    pub fn as_mut_ptr<T>(self) -> *mut T {
        self.as_u64() as *mut T
    }

    #[inline]
    pub fn align_up(self, align: impl Into<u64>) -> Self {
        Self::new(align_up(self.0, align.into()))
    }

    #[inline]
    pub fn align_down(self, align: impl Into<u64>) -> Self {
        Self(align_down(self.0, align.into()))
    }

    #[inline]
    pub fn is_aligned(self, align: impl Into<u64>) -> bool {
        self.align_down(align) == self
    }
}

impl Default for PhysicalAddress {
    fn default() -> Self {
        Self::zero()
    }
}

impl fmt::Debug for PhysicalAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "PhysicalAddress({:#x})", self.0)
    }
}

impl fmt::Binary for PhysicalAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::LowerHex for PhysicalAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::UpperHex for PhysicalAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl fmt::Octal for PhysicalAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.fmt(f)
    }
}

impl<U: Into<u64>> Add<U> for PhysicalAddress {
    type Output = Self;
    fn add(self, rhs: U) -> Self::Output {
        Self::Output::new(self.0 + rhs.into())
    }
}

impl<U: Into<u64>> AddAssign<U> for PhysicalAddress {
    fn add_assign(&mut self, rhs: U) {
        *self = *self + rhs;
    }
}

impl<U: Into<u64>> Sub<U> for PhysicalAddress {
    type Output = Self;
    fn sub(self, rhs: U) -> Self::Output {
        Self::Output::new(self.0.checked_sub(rhs.into()).unwrap())
    }
}

impl<U: Into<u64>> SubAssign<U> for PhysicalAddress {
    fn sub_assign(&mut self, rhs: U) {
        *self = *self - rhs;
    }
}

impl Sub<PhysicalAddress> for PhysicalAddress {
    type Output = u64;
    fn sub(self, rhs: PhysicalAddress) -> Self::Output {
        self.as_u64().checked_sub(rhs.as_u64()).unwrap()
    }
}
