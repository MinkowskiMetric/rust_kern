use super::{align_down, align_up};
use super::{PageOffset, PageTableIndex};
use bit_field::BitField;
use core::fmt;
use core::ops::{Add, AddAssign, Sub, SubAssign};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(transparent)]
pub struct VirtualAddress(u64);

#[derive(Debug)]
pub struct VirtualAddressNotValid(u64);

impl VirtualAddress {
    #[inline]
    pub fn new(addr: u64) -> Self {
        Self::try_new(addr).expect("Invalid virtual address")
    }

    #[inline]
    pub fn try_new(addr: u64) -> Result<Self, VirtualAddressNotValid> {
        match addr.get_bits(47..64) {
            0 | 0x1ffff => Ok(Self(addr)),     // Address is already canonical
            1 => Ok(Self::new_truncate(addr)), // Address can be made canonical
            other => Err(VirtualAddressNotValid(other)), // address is not valid
        }
    }

    #[inline]
    pub const fn new_truncate(addr: u64) -> Self {
        Self(((addr << 16) as i64 >> 16) as u64)
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
        Self(align_up(self.0, align.into()))
    }

    #[inline]
    pub fn align_down(self, align: impl Into<u64>) -> Self {
        Self(align_down(self.0, align.into()))
    }

    #[inline]
    pub fn is_aligned(self, align: impl Into<u64>) -> bool {
        self.align_down(align) == self
    }

    #[inline]
    pub fn checked_add(self, val: impl Into<u64>) -> Option<Self> {
        self.as_u64()
            .checked_add(val.into())
            .and_then(|addr| Self::try_new(addr).ok())
    }

    #[inline]
    pub fn checked_sub(self, val: impl Into<u64>) -> Option<Self> {
        self.as_u64()
            .checked_sub(val.into())
            .and_then(|addr| Self::try_new(addr).ok())
    }

    pub const fn page_offset(self) -> PageOffset {
        PageOffset::new_truncate(self.0 as u16)
    }

    pub const fn p1_index(self) -> PageTableIndex {
        PageTableIndex::new_truncate((self.0 >> 12) as u16)
    }

    pub const fn p2_index(self) -> PageTableIndex {
        PageTableIndex::new_truncate((self.0 >> 12 >> 9) as u16)
    }

    pub const fn p3_index(self) -> PageTableIndex {
        PageTableIndex::new_truncate((self.0 >> 12 >> 9 >> 9) as u16)
    }

    pub const fn p4_index(self) -> PageTableIndex {
        PageTableIndex::new_truncate((self.0 >> 12 >> 9 >> 9 >> 9) as u16)
    }
}

impl Default for VirtualAddress {
    fn default() -> Self {
        Self::zero()
    }
}

impl fmt::Debug for VirtualAddress {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "VirtualAddress({:#x})", self.0)
    }
}

impl<U: Into<u64>> Add<U> for VirtualAddress {
    type Output = Self;
    fn add(self, rhs: U) -> Self::Output {
        Self::Output::new(self.0 + rhs.into())
    }
}

impl<U: Into<u64>> AddAssign<U> for VirtualAddress {
    fn add_assign(&mut self, rhs: U) {
        *self = *self + rhs;
    }
}

impl<U: Into<u64>> Sub<U> for VirtualAddress {
    type Output = Self;
    fn sub(self, rhs: U) -> Self::Output {
        Self::Output::new(self.0.checked_sub(rhs.into()).unwrap())
    }
}

impl<U: Into<u64>> SubAssign<U> for VirtualAddress {
    fn sub_assign(&mut self, rhs: U) {
        *self = *self - rhs;
    }
}

impl Sub<VirtualAddress> for VirtualAddress {
    type Output = u64;
    fn sub(self, rhs: VirtualAddress) -> Self::Output {
        self.as_u64().checked_sub(rhs.as_u64()).unwrap()
    }
}
