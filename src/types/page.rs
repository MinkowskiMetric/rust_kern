use crate::types::{
    NotGiantPageSize, PageSize, PageTableIndex, Size1GiB, Size2MiB, Size4KiB, VirtualAddress,
};
use core::{
    fmt,
    iter::Step,
    marker::PhantomData,
    ops::{Add, AddAssign, Sub, SubAssign},
};

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(C)]
pub struct Page<S: PageSize = Size4KiB> {
    start_address: VirtualAddress,
    size: PhantomData<S>,
}

impl<S: PageSize> Page<S> {
    pub const SIZE: u64 = S::SIZE;

    #[inline]
    pub fn from_start_address(addr: VirtualAddress) -> Result<Self, ()> {
        if addr.is_aligned(S::SIZE) {
            Ok(Self::containing_address(addr))
        } else {
            Err(())
        }
    }

    #[inline]
    pub const unsafe fn from_start_address_unchecked(start_address: VirtualAddress) -> Self {
        Self {
            start_address,
            size: PhantomData,
        }
    }

    #[inline]
    pub fn containing_address(address: VirtualAddress) -> Self {
        Self {
            start_address: address.align_down(Self::SIZE),
            size: PhantomData,
        }
    }

    #[inline]
    pub const fn start_address(self) -> VirtualAddress {
        self.start_address
    }

    #[inline]
    pub const fn size(self) -> u64 {
        Self::SIZE
    }

    #[inline]
    pub const fn p4_index(self) -> PageTableIndex {
        self.start_address().p4_index()
    }

    #[inline]
    pub const fn p3_index(self) -> PageTableIndex {
        self.start_address().p3_index()
    }
}

impl<S: NotGiantPageSize> Page<S> {
    #[inline]
    pub const fn p2_index(self) -> PageTableIndex {
        self.start_address().p2_index()
    }
}

impl Page<Size1GiB> {
    #[inline]
    pub fn from_page_table_indices_1gib(
        p4_index: PageTableIndex,
        p3_index: PageTableIndex,
    ) -> Self {
        use bit_field::BitField;

        let mut addr = 0;
        addr.set_bits(39..48, u64::from(p4_index));
        addr.set_bits(30..39, u64::from(p3_index));
        Page::containing_address(VirtualAddress::new(addr))
    }
}

impl Page<Size2MiB> {
    #[inline]
    pub fn from_page_table_indices_2mib(
        p4_index: PageTableIndex,
        p3_index: PageTableIndex,
        p2_index: PageTableIndex,
    ) -> Self {
        use bit_field::BitField;

        let mut addr = 0;
        addr.set_bits(39..48, u64::from(p4_index));
        addr.set_bits(30..39, u64::from(p3_index));
        addr.set_bits(21..30, u64::from(p2_index));
        Page::containing_address(VirtualAddress::new(addr))
    }
}

impl Page<Size4KiB> {
    #[inline]
    pub fn from_page_table_indices(
        p4_index: PageTableIndex,
        p3_index: PageTableIndex,
        p2_index: PageTableIndex,
        p1_index: PageTableIndex,
    ) -> Self {
        use bit_field::BitField;

        let mut addr = 0;
        addr.set_bits(39..48, u64::from(p4_index));
        addr.set_bits(30..39, u64::from(p3_index));
        addr.set_bits(21..30, u64::from(p2_index));
        addr.set_bits(12..21, u64::from(p1_index));
        Page::containing_address(VirtualAddress::new(addr))
    }

    #[inline]
    pub const fn p1_index(self) -> PageTableIndex {
        self.start_address().p1_index()
    }
}

impl<S: PageSize> fmt::Debug for Page<S> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!(
            "Page[{}]({:#x})",
            S::SIZE_AS_DEBUG_STR,
            self.start_address().as_u64()
        ))
    }
}

impl<S: PageSize, U: Into<u64>> Add<U> for Page<S> {
    type Output = Self;

    fn add(self, rhs: U) -> Self::Output {
        Self::containing_address(self.start_address() + rhs.into() * Self::SIZE)
    }
}

impl<S: PageSize, U: Into<u64>> AddAssign<U> for Page<S> {
    fn add_assign(&mut self, rhs: U) {
        *self = *self + rhs;
    }
}

impl<S: PageSize, U: Into<u64>> Sub<U> for Page<S> {
    type Output = Self;

    fn sub(self, rhs: U) -> Self::Output {
        Self::containing_address(self.start_address() - rhs.into() * Self::SIZE)
    }
}

impl<S: PageSize, U: Into<u64>> SubAssign<U> for Page<S> {
    fn sub_assign(&mut self, rhs: U) {
        *self = *self - rhs;
    }
}

impl<S: PageSize> Sub<Self> for Page<S> {
    type Output = u64;

    fn sub(self, rhs: Self) -> Self::Output {
        (self.start_address() - rhs.start_address()) / Self::SIZE
    }
}

impl<S: PageSize> Sub<Self> for &Page<S> {
    type Output = u64;

    fn sub(self, rhs: Self) -> Self::Output {
        *self - *rhs
    }
}

unsafe impl<S: PageSize> Step for Page<S> {
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
    pub fn test_page_ranges() {
        let page_size = Size4KiB::SIZE;
        let number = 1000;

        let start_addr = VirtualAddress::new(0xdead_beef);
        let start: Page = Page::containing_address(start_addr);
        let end = start + number;

        let mut range = (start..end).into_iter();
        for i in 0..number {
            assert_eq!(
                range.next(),
                Some(Page::containing_address(start_addr + page_size * i))
            );
        }
    }
}
