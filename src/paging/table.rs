use super::page_entry::{PresentPageFlags, RawNotPresentPte, RawPresentPte, RawPte};
use super::Result;
use super::{phys_to_virt, phys_to_virt_mut};
use crate::physmem;
use crate::physmem::Frame;
use core::convert::{Infallible, TryFrom};
use core::fmt;
use core::marker::PhantomData;
use core::num::TryFromIntError;
use core::ops::{Index, IndexMut};

const ENTRY_COUNT: u16 = 512;

#[derive(Debug, Clone, Copy)]
pub struct InvalidPageTableIndex(());

impl From<Infallible> for InvalidPageTableIndex {
    fn from(_: Infallible) -> Self {
        Self(())
    }
}

impl From<TryFromIntError> for InvalidPageTableIndex {
    fn from(_: TryFromIntError) -> Self {
        Self(())
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct PageTableIndex(u16);

impl PageTableIndex {
    pub const fn new_truncate(val: u16) -> Self {
        Self(val % ENTRY_COUNT)
    }

    pub const unsafe fn new_unchecked(val: u16) -> Self {
        Self(val)
    }
}

impl fmt::Debug for PageTableIndex {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        f.write_fmt(format_args!("PageTableIndex({:#x})", self.0))
    }
}

macro_rules! page_table_index_conversions (
    () => { };
    ($t:ty $(, $others:ty)* $(,)?) => {
        page_table_index_conversions!($($others),*);

        impl TryFrom<$t> for PageTableIndex {
            type Error = InvalidPageTableIndex;

            fn try_from(val: $t) -> core::result::Result<Self, Self::Error> {
                let val = u16::try_from(val)?;

                if val < ENTRY_COUNT {
                    Ok(Self(val))
                } else {
                    Err(InvalidPageTableIndex(()))
                }
            }
        }

        impl From<PageTableIndex> for $t {
            fn from(val: PageTableIndex) -> Self {
                val.0.into()
            }
        }
    }
);

page_table_index_conversions!(u16, u32, u64, usize,);

#[derive(Debug, Clone, Copy)]
pub struct InvalidPageOffset(());

impl From<Infallible> for InvalidPageOffset {
    fn from(_: Infallible) -> Self {
        Self(())
    }
}

impl From<TryFromIntError> for InvalidPageOffset {
    fn from(_: TryFromIntError) -> Self {
        Self(())
    }
}

pub const fn p1_index(va: usize) -> PageTableIndex {
    PageTableIndex::new_truncate((va >> 12) as u16)
}

pub const fn p2_index(va: usize) -> PageTableIndex {
    PageTableIndex::new_truncate((va >> 12 >> 9) as u16)
}

pub const fn p3_index(va: usize) -> PageTableIndex {
    PageTableIndex::new_truncate((va >> 12 >> 9 >> 9) as u16)
}

pub const fn p4_index(va: usize) -> PageTableIndex {
    PageTableIndex::new_truncate((va >> 12 >> 9 >> 9 >> 9) as u16)
}

pub trait PageTableLevel {}

pub trait HierarchyLevel: PageTableLevel {
    type NextLevel: PageTableLevel;
}

pub enum L4 {}
pub enum L3 {}
pub enum L2 {}
pub enum L1 {}

impl PageTableLevel for L4 {}
impl PageTableLevel for L3 {}
impl PageTableLevel for L2 {}
impl PageTableLevel for L1 {}

impl HierarchyLevel for L4 {
    type NextLevel = L3;
}

impl HierarchyLevel for L3 {
    type NextLevel = L2;
}

impl HierarchyLevel for L2 {
    type NextLevel = L1;
}

#[repr(C)]
#[repr(align(4096))]
pub struct PageTable<L: PageTableLevel>([RawPte; ENTRY_COUNT as usize], PhantomData<L>);

impl<L: PageTableLevel> PageTable<L> {
    pub unsafe fn at_virtual_address(addr: usize) -> &'static Self {
        &*(addr as *const Self)
    }

    pub unsafe fn at_virtual_address_mut(addr: usize) -> &'static mut Self {
        &mut *(addr as *mut Self)
    }

    pub fn iter<'a>(&'a self) -> core::slice::Iter<'a, RawPte> {
        self.0.iter()
    }

    pub fn iter_mut<'a>(&'a mut self) -> core::slice::IterMut<'a, RawPte> {
        self.0.iter_mut()
    }

    pub fn zero(&mut self) {
        for entry in self.iter_mut() {
            *entry = RawPte::unused();
        }
    }
}

impl<L: 'static + HierarchyLevel> PageTable<L> {
    pub fn create_next_table<'a>(
        &'a mut self,
        index: PageTableIndex,
    ) -> Result<&'a mut PageTable<L::NextLevel>> {
        if self.next_table_frame(index).is_none() {
            assert!(
                !self[index]
                    .present()
                    .map(|present_pte| present_pte.is_huge())
                    .unwrap_or(false),
                "Huge page not supported"
            );
            let new_page_table = physmem::allocate_frame()
                .expect("Failed to allocate frame in boot_create_next_table");
            self[index] = RawPresentPte::from_frame_and_flags(
                new_page_table,
                PresentPageFlags::WRITABLE | PresentPageFlags::USER_ACCESSIBLE,
            )
            .into();
        }

        Ok(self.next_table_mut(index).unwrap())
    }

    pub fn next_table<'a>(&'a self, index: PageTableIndex) -> Option<&'a PageTable<L::NextLevel>> {
        self.next_table_frame(index)
            .map(|f| unsafe { &*phys_to_virt(f.physical_address() as usize) })
    }

    pub fn next_table_mut<'a>(
        &'a mut self,
        index: PageTableIndex,
    ) -> Option<&'a mut PageTable<L::NextLevel>> {
        self.next_table_frame(index)
            .map(|f| unsafe { &mut *phys_to_virt_mut(f.physical_address() as usize) })
    }

    pub fn next_table_frame(&self, index: PageTableIndex) -> Option<Frame> {
        self[index]
            .present()
            .ok()
            .map(|present_pte| present_pte.frame())
    }
}

impl PageTable<L1> {
    pub fn set_present(&mut self, index: PageTableIndex, new_pte: impl Into<RawPresentPte>) {
        self.do_set_pte(index, new_pte.into());
    }

    pub fn set_not_present(&mut self, index: PageTableIndex, new_pte: impl Into<RawNotPresentPte>) {
        self.do_set_pte(index, new_pte.into());
    }

    fn do_set_pte(&mut self, index: PageTableIndex, new_pte: impl Into<RawPte>) {
        let pte = &self[index];

        // We should only be doing this for not present pages
        assert!(!pte.is_present());
        self.0[usize::from(index)] = new_pte.into();
    }
}

impl<L: PageTableLevel> Index<PageTableIndex> for PageTable<L> {
    type Output = RawPte;

    fn index(&self, index: PageTableIndex) -> &Self::Output {
        &self.0[usize::from(index)]
    }
}

impl<L: PageTableLevel> IndexMut<PageTableIndex> for PageTable<L> {
    fn index_mut(&mut self, index: PageTableIndex) -> &mut Self::Output {
        &mut self.0[usize::from(index)]
    }
}
