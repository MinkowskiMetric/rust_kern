use crate::types::{PageSize, PhysicalAddress, Size4KiB};
use core::{
    fmt,
    iter::Step,
    marker::PhantomData,
    ops::{Add, AddAssign, Sub, SubAssign},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct Frame<S: PageSize = Size4KiB> {
    start_address: PhysicalAddress,
    size: PhantomData<S>,
}

impl<S: PageSize> Frame<S> {
    pub const SIZE: u64 = S::SIZE;

    #[inline]
    pub fn from_start_address(addr: PhysicalAddress) -> Result<Self, ()> {
        if addr.is_aligned(S::SIZE) {
            Ok(Self::containing_address(addr))
        } else {
            Err(())
        }
    }

    #[inline]
    pub const unsafe fn from_start_address_unchecked(start_address: PhysicalAddress) -> Self {
        Self {
            start_address,
            size: PhantomData,
        }
    }

    #[inline]
    pub fn containing_address(address: PhysicalAddress) -> Self {
        Self {
            start_address: address.align_down(Self::SIZE),
            size: PhantomData,
        }
    }

    #[inline]
    pub const fn start_address(self) -> PhysicalAddress {
        self.start_address
    }

    #[inline]
    pub const fn size(self) -> u64 {
        Self::SIZE
    }
}

impl<S: PageSize> fmt::Debug for Frame<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!(
            "Frame[{}]({:#x})",
            S::SIZE_AS_DEBUG_STR,
            self.start_address().as_u64()
        ))
    }
}

impl<S: PageSize, U: Into<u64>> Add<U> for Frame<S> {
    type Output = Self;

    fn add(self, rhs: U) -> Self::Output {
        Self::containing_address(self.start_address() + rhs.into() * Self::SIZE)
    }
}

impl<S: PageSize, U: Into<u64>> AddAssign<U> for Frame<S> {
    fn add_assign(&mut self, rhs: U) {
        *self = *self + rhs;
    }
}

impl<S: PageSize, U: Into<u64>> Sub<U> for Frame<S> {
    type Output = Self;

    fn sub(self, rhs: U) -> Self::Output {
        Self::containing_address(self.start_address() - rhs.into() * Self::SIZE)
    }
}

impl<S: PageSize, U: Into<u64>> SubAssign<U> for Frame<S> {
    fn sub_assign(&mut self, rhs: U) {
        *self = *self - rhs;
    }
}

impl<S: PageSize> Sub<Self> for Frame<S> {
    type Output = u64;

    fn sub(self, rhs: Self) -> Self::Output {
        (self.start_address() - rhs.start_address()) / Self::SIZE
    }
}

impl<S: PageSize> Sub<Self> for &Frame<S> {
    type Output = u64;

    fn sub(self, rhs: Self) -> Self::Output {
        *self - *rhs
    }
}

unsafe impl<S: PageSize> Step for Frame<S> {
    fn steps_between(start: &Self, end: &Self) -> Option<usize> {
        if start.start_address() <= end.start_address() {
            Some((end - start) as usize)
        } else {
            None
        }
    }

    fn forward_checked(start: Self, count: usize) -> Option<Self> {
        Some(start + count as u64)
    }

    fn backward_checked(start: Self, count: usize) -> Option<Self> {
        Some(start - count as u64)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test_case]
    pub fn test_frame_ranges() {
        let page_size = Size4KiB::SIZE;
        let number = 1000;

        let start_addr = PhysicalAddress::new(0xdead_beef);
        let start: Frame = Frame::containing_address(start_addr);
        let end = start + number;

        let mut range = (start..end).into_iter();
        for i in 0..number {
            assert_eq!(
                range.next(),
                Some(Frame::containing_address(start_addr + page_size * i))
            );
        }
    }
}
